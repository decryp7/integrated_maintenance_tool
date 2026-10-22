import { inject, Injectable, InjectionToken, signal } from '@angular/core';

/** Where query history is kept. Replaceable in tests; the default is `localStorage` (a convenience only). */
export interface HistoryStore {
  load(): string[];
  save(entries: string[]): void;
}

const KEY = 'debrief.queryHistory.v1';
export const MAX_HISTORY = 50;

export class LocalHistoryStore implements HistoryStore {
  load(): string[] {
    try {
      const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? '[]');
      return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string').slice(0, MAX_HISTORY) : [];
    } catch {
      return [];
    }
  }
  save(entries: string[]): void {
    try {
      localStorage.setItem(KEY, JSON.stringify(entries));
    } catch {
      /* storage unavailable: history is a convenience only */
    }
  }
}

export const HISTORY_STORE = new InjectionToken<HistoryStore>('HISTORY_STORE', { providedIn: 'root', factory: () => new LocalHistoryStore() });

/** Most-recent-first list of queries that ran successfully. */
@Injectable({ providedIn: 'root' })
export class QueryHistory {
  private readonly store = inject(HISTORY_STORE);
  private readonly list = signal<string[]>(this.store.load());
  readonly entries = this.list.asReadonly();

  add(text: string): void {
    const t = text.trim();
    if (!t) return;
    const next = [t, ...this.list().filter((e) => e !== t)].slice(0, MAX_HISTORY);
    this.list.set(next);
    this.store.save(next);
  }

  clear(): void {
    this.list.set([]);
    this.store.save([]);
  }
}
