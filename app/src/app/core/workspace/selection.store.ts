import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { RecordView } from '../api.types';

interface SelectionState {
  /** The record the inspector shows. */
  selected: RecordView | null;
  /** Request for the grid to scroll to (and select) a record. */
  anchor: { id: number; nonce: number } | null;
  /** The last anchor is hidden by the current filters. */
  anchorMissed: boolean;
  /** Events matching the query (reported by the grid). */
  matchTotal: number | null;
}

/** The selected record, jump-to-record requests and the match count the grid reports. */
export const SelectionStore = signalStore(
  { providedIn: 'root' },
  withState<SelectionState>({ selected: null, anchor: null, anchorMissed: false, matchTotal: null }),
  withMethods((store) => {
    let anchorNonce = 0;
    return {
      select: (selected: RecordView | null) => patchState(store, { selected, anchorMissed: false }),
      jumpTo(id: number) {
        patchState(store, { anchor: { id, nonce: ++anchorNonce }, anchorMissed: false });
      },
      anchorResolved(row: RecordView | null) {
        patchState(store, row ? { selected: row, anchorMissed: false } : { anchorMissed: true });
      },
      setMatchTotal: (matchTotal: number | null) => patchState(store, { matchTotal }),
    };
  }),
);
