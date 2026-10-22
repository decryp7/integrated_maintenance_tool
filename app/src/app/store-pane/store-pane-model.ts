import { FieldStorage, MaintenanceAction, StoreActivity, StoreMemory } from '../core/api.types';

/** The Store pane's view id (View ▸ Store). */
export const STORE_VIEW = 'store';

/** A bar of the Memory section: a value against a budget. */
export interface Gauge {
  label: string;
  value: number;
  max: number;
  /** 0..100, for the bar's width. */
  percent: number;
  /** At or above 85% of the budget. */
  near: boolean;
}

export function gauge(label: string, value: number, max: number): Gauge {
  const percent = max > 0 ? Math.min(100, Math.round((value / max) * 100)) : 0;
  return { label, value, max, percent, near: percent >= 85 };
}

/** The Memory section's bars: the worker against the budget that applies now, DuckDB against its limit. */
export function memoryGauges(m: StoreMemory): Gauge[] {
  const out: Gauge[] = [];
  if (m.worker) {
    const budget = m.importing ? m.importBudgetBytes : m.queryBudgetBytes;
    const label = m.importing
      ? $localize`:@@store.memory.workerImporting:Store worker (importing budget)`
      : $localize`:@@store.memory.workerQuerying:Store worker (query budget)`;
    out.push(gauge(label, m.worker.rssBytes, budget));
  }
  out.push(gauge($localize`:@@store.memory.duck:DuckDB`, m.duckUsedBytes, m.duckLimitBytes));
  return out;
}

export function activityLabel(a: StoreActivity): string {
  switch (a) {
    case 'ready':
      return $localize`:@@store.activity.ready:Ready`;
    case 'importing':
      return $localize`:@@store.activity.importing:Importing`;
    case 'indexing':
      return $localize`:@@store.activity.indexing:Indexing`;
  }
}

export function storageLabel(s: FieldStorage): string {
  switch (s) {
    case 'declared':
      return $localize`:@@store.storage.declared:Declared column`;
    case 'promoted':
      return $localize`:@@store.storage.promoted:Own column`;
    case 'partly':
      return $localize`:@@store.storage.partly:Own column in some segments`;
    case 'overflow':
      return $localize`:@@store.storage.overflow:Overflow map (slower to filter)`;
  }
}

/** The confirmation each maintenance action asks for. */
export function maintenanceText(a: MaintenanceAction): { title: string; message: string; confirm: string } {
  switch (a) {
    case 'compact':
      return {
        title: $localize`:@@store.maintain.compactTitle:Compact now`,
        message: $localize`:@@store.maintain.compactMessage:Merge small segments and drop the rows of removed sources now? The store rewrites those segments; reading goes on meanwhile.`,
        confirm: $localize`:@@store.maintain.compactConfirm:Compact`,
      };
    case 'rebuildIndex':
      return {
        title: $localize`:@@store.maintain.rebuildTitle:Rebuild index`,
        message: $localize`:@@store.maintain.rebuildMessage:Throw the full-text index away and index every event again? Word search finds fewer events until it caught up.`,
        confirm: $localize`:@@store.maintain.rebuildConfirm:Rebuild`,
      };
    case 'clearCachedAnalyses':
      return {
        title: $localize`:@@store.maintain.clearTitle:Clear cached analyses`,
        message: $localize`:@@store.maintain.clearMessage:Forget the cached analyses? They are computed again when you next open them.`,
        confirm: $localize`:@@store.maintain.clearConfirm:Clear`,
      };
  }
}
