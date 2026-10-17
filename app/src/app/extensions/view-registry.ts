import { computed, inject, Injectable } from '@angular/core';
import { WorkspaceSummary } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { flattenContributions, IMT_VIEWS, ImtView } from './tokens';

/** Every registered view and which of them the current workspace offers. */
@Injectable({ providedIn: 'root' })
export class ViewRegistry {
  private readonly store = inject(WorkspaceStore);
  readonly all: readonly ImtView[] = dedupe(flattenContributions<ImtView>(inject(IMT_VIEWS, { optional: true })));

  /** Views whose `when` holds for the current workspace (views without `when` always). */
  readonly available = computed(() => {
    const summary = this.store.summary();
    return this.all.filter((v) => isViewAvailable(v, summary));
  });

  find(id: string): ImtView | undefined {
    return this.all.find((v) => v.id === id);
  }
}

export function isViewAvailable(view: ImtView, summary: WorkspaceSummary | null): boolean {
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
