import { Anomaly } from '../core/api.types';
import { DEFAULT_ANOMALY_SORT, nextSort, sortAnomalies } from './anomaly-sort';

const a = (kind: string, instance: string | null, from: number, score: number, message = kind): Anomaly =>
  ({ kind, instance, from, to: from, score, message, sampleIds: [] }) as unknown as Anomaly;

describe('anomaly sort (K2)', () => {
  const list = [a('crash', 'WEB02', 30, 5), a('silence', null, 10, 9), a('errorSpike', 'WEB01', 20, 7)];

  it('sorts by each column, ascending or descending, without touching the input', () => {
    const copy = [...list];
    expect(sortAnomalies(list, DEFAULT_ANOMALY_SORT).map((x) => x.from)).toEqual([10, 20, 30]);
    expect(sortAnomalies(list, { column: 'score', descending: true }).map((x) => x.score)).toEqual([9, 7, 5]);
    expect(sortAnomalies(list, { column: 'instance', descending: false }).map((x) => x.instance)).toEqual([null, 'WEB01', 'WEB02']);
    expect(sortAnomalies(list, { column: 'kind', descending: false }).map((x) => x.kind)).toEqual(['crash', 'errorSpike', 'silence']);
    expect(sortAnomalies(list, { column: 'message', descending: true }).map((x) => x.message)).toEqual(['silence', 'errorSpike', 'crash']);
    expect(list).toEqual(copy);
  });

  it('a new column starts ascending (score: highest first) and the same column reverses', () => {
    expect(nextSort(DEFAULT_ANOMALY_SORT, 'kind')).toEqual({ column: 'kind', descending: false });
    expect(nextSort(DEFAULT_ANOMALY_SORT, 'score')).toEqual({ column: 'score', descending: true });
    expect(nextSort({ column: 'kind', descending: false }, 'kind')).toEqual({ column: 'kind', descending: true });
  });
});
