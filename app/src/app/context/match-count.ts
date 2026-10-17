import { DestroyRef, effect, inject, Injectable, signal, Signal, untracked } from '@angular/core';
import { ImtBackend } from '../core/backend';
import { Condition } from '../core/profile.model';
import { EMPTY_QUERY } from '../core/workspace.store';
import { asFilters } from './rule-builders';

/** Counts the events matching conditions (a read-only query, no side effects). */
@Injectable({ providedIn: 'root' })
export class MatchCounter {
  private readonly backend = inject(ImtBackend);

  async count(conditions: readonly Condition[]): Promise<number> {
    const query = { ...EMPTY_QUERY, filters: asFilters(conditions) };
    const page = await this.backend.queryPage({ query, offset: 0, limit: 1, descending: false, anchor: null });
    return page.total;
  }
}

/**
 * A signal with the number of events matching the (changing) conditions, recomputed after a short
 * pause in typing. null while it is being computed or when the query failed. Call in an injection context.
 */
export function matchCount(conditions: () => readonly Condition[], delayMs = 250): Signal<number | null> {
  const counter = inject(MatchCounter);
  const out = signal<number | null>(null);
  let seq = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  effect(() => {
    const current = conditions();
    clearTimeout(timer);
    const mine = ++seq;
    untracked(() => out.set(null));
    timer = setTimeout(async () => {
      try {
        const n = await counter.count(current);
        if (mine === seq) out.set(n);
      } catch {
        if (mine === seq) out.set(null);
      }
    }, delayMs);
  });
  inject(DestroyRef).onDestroy(() => clearTimeout(timer));
  return out;
}
