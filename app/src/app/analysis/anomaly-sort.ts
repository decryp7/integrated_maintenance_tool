import { Anomaly } from '../core/api.types';

/** The columns of the anomalies table. */
export type AnomalyColumn = 'kind' | 'instance' | 'time' | 'message' | 'score';

export interface AnomalySort {
  column: AnomalyColumn;
  descending: boolean;
}

/** Engine order (by time) is the default; a header click sorts, a second click reverses. */
export const DEFAULT_ANOMALY_SORT: AnomalySort = { column: 'time', descending: false };

/** The next sort after clicking `column`'s header: a new column starts ascending (score: highest first), the same one reverses. */
export function nextSort(current: AnomalySort, column: AnomalyColumn): AnomalySort {
  if (current.column === column) return { column, descending: !current.descending };
  return { column, descending: column === 'score' };
}

/** The anomalies in display order. Pure and stable; never mutates the engine's list. */
export function sortAnomalies(list: readonly Anomaly[], sort: AnomalySort): Anomaly[] {
  const key = (a: Anomaly): string | number => {
    switch (sort.column) {
      case 'kind':
        return a.kind;
      case 'instance':
        return a.instance ?? '';
      case 'message':
        return a.message;
      case 'score':
        return a.score;
      default:
        return a.from;
    }
  };
  const dir = sort.descending ? -1 : 1;
  return [...list].sort((a, b) => {
    const x = key(a);
    const y = key(b);
    return (typeof x === 'number' && typeof y === 'number' ? x - y : String(x).localeCompare(String(y))) * dir;
  });
}
