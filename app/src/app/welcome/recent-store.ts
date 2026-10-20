import { inject, InjectionToken } from '@angular/core';
import { RecentService } from '../core/recent/recent.service';

/** The folders and files the user opened, newest first. Implementations must never throw. */
export interface RecentStore {
  /** Reads a signal where the implementation has one, so `computed` follows changes. */
  list(): string[];
  /** The UI opened these (Rust records them itself; see `RecentBackend.opened`). */
  add(paths: string[]): void;
  clear(): void;
}

/** The recent files of Rust's list (see `RecentService`). */
export class BackendRecentStore implements RecentStore {
  constructor(private readonly recent: RecentService) {}

  list(): string[] {
    return this.recent.files();
  }
  add(paths: string[]): void {
    void this.recent.opened(paths);
  }
  clear(): void {
    void this.recent.clear('files');
  }
}

export const RECENT_STORE = new InjectionToken<RecentStore>('RECENT_STORE', {
  providedIn: 'root',
  factory: () => new BackendRecentStore(inject(RecentService)),
});
