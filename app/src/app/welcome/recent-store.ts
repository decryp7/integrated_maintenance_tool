import { InjectionToken } from '@angular/core';

/** Remembers the folders and files the user opened, newest first. Implementations must never throw. */
export interface RecentStore {
  list(): string[];
  /** Puts paths at the front (deduplicated, capped). */
  add(paths: string[]): void;
  clear(): void;
}

export const RECENT_KEY = 'imt.recent';
export const RECENT_VERSION = 1;
export const RECENT_MAX = 8;

/** `localStorage` implementation: a versioned JSON envelope. */
export class LocalStorageRecentStore implements RecentStore {
  constructor(
    private readonly key = RECENT_KEY,
    private readonly max = RECENT_MAX,
  ) {}

  list(): string[] {
    try {
      const parsed = JSON.parse(localStorage.getItem(this.key) ?? 'null') as { version?: number; paths?: unknown } | null;
      if (!parsed || parsed.version !== RECENT_VERSION || !Array.isArray(parsed.paths)) return [];
      return parsed.paths.filter((p): p is string => typeof p === 'string').slice(0, this.max);
    } catch {
      return [];
    }
  }

  add(paths: string[]): void {
    const next = [...paths, ...this.list().filter((p) => !paths.includes(p))].slice(0, this.max);
    this.write(next);
  }

  clear(): void {
    this.write([]);
  }

  private write(paths: string[]): void {
    try {
      localStorage.setItem(this.key, JSON.stringify({ version: RECENT_VERSION, paths }));
    } catch {
      /* storage unavailable: recents are a convenience only */
    }
  }
}

export const RECENT_STORE = new InjectionToken<RecentStore>('RECENT_STORE', {
  providedIn: 'root',
  factory: () => new LocalStorageRecentStore(),
});
