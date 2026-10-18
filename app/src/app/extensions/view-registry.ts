import { computed, inject, Injectable } from '@angular/core';
import { WorkspaceSummary } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { withInjectedFaults } from '../core/activity/fault-injection';
import { flattenContributions, IMT_VIEWS, ImtView } from './tokens';

/** Every registered view and which of them the current workspace offers. */
@Injectable({ providedIn: 'root' })
export class ViewRegistry {
  private readonly store = inject(WorkspaceStore);
  // `withInjectedFaults`: the error-capture test hook (browser builds only, see core/activity/fault-injection.ts).
  readonly all: readonly ImtView[] = withInjectedFaults(dedupe(flattenContributions<ImtView>(inject(IMT_VIEWS, { optional: true }))));

  /** Views whose `when` holds for the current workspace (views without `when` always). */
  readonly available = computed(() => {
    const summary = this.store.summary();
    return this.all.filter((v) => isViewAvailable(v, summary));
  });

  /** The view by id; `<view id>#<key>` (one more panel of a view that allows several) finds that view. */
  find(id: string): ImtView | undefined {
    const exact = this.all.find((v) => v.id === id);
    if (exact) return exact;
    const at = id.indexOf('#');
    if (at <= 0 || at === id.length - 1) return undefined;
    const base = id.slice(0, at);
    return this.all.find((v) => v.id === base && v.instances === true);
  }
}

export function isViewAvailable(view: ImtView, summary: WorkspaceSummary | null): boolean {
  try {
    if (view.enabled && !view.enabled()) return false;
  } catch {
    return false;
  }
  if (!view.when) return true;
  if (!summary) return false;
  try {
    return view.when(summary);
  } catch {
    return false;
  }
}

function dedupe(views: ImtView[]): ImtView[] {
  const seen = new Set<string>();
  return views.filter((v) => !seen.has(v.id) && !!seen.add(v.id));
}
