import { computed } from '@angular/core';
import { signalStoreFeature, withComputed, withState } from '@ngrx/signals';

/*
 * NOTE (rule 39): this feature is not Debrief-specific. When a second Paddock app needs it, move it to `paddock-ui`
 * instead of copying it.
 */

/**
 * Where a read (or any one request) stands. `error` carries the message to show, so "loading" and "failed" cannot be
 * true together and a failure always has text (rule 30).
 */
export type RequestStatus = 'idle' | 'loading' | 'loaded' | { readonly error: string };

/** State added by `withRequestStatus`. */
export interface RequestStatusState {
  requestStatus: RequestStatus;
}

/** A status update for `patchState(store, setLoading())`; combine with other changes in one call. */
export const setLoading = (): RequestStatusState => ({ requestStatus: 'loading' });
/** The request finished; the data it fetched is set in the same `patchState`. */
export const setLoaded = (): RequestStatusState => ({ requestStatus: 'loaded' });
/** The request failed; `message` is what the user should read. */
export const setError = (message: string): RequestStatusState => ({ requestStatus: { error: message } });
/** Back to "nothing requested yet" (e.g. after a reset). */
export const setIdle = (): RequestStatusState => ({ requestStatus: 'idle' });

/** The error message of a status, or null when it is not an error. */
export function statusError(status: RequestStatus): string | null {
  return typeof status === 'object' ? status.error : null;
}

/**
 * Adds `requestStatus` (idle | loading | loaded | error) and the computed `isIdle`, `isLoading`, `isLoaded`, `isError`
 * and `error` (message or null). Change it with `setLoading()`, `setLoaded()`, `setError(message)` inside the store's
 * own methods.
 */
export function withRequestStatus() {
  return signalStoreFeature(
    withState<RequestStatusState>({ requestStatus: 'idle' }),
    withComputed(({ requestStatus }) => ({
      isIdle: computed(() => requestStatus() === 'idle'),
      isLoading: computed(() => requestStatus() === 'loading'),
      isLoaded: computed(() => requestStatus() === 'loaded'),
      isError: computed(() => typeof requestStatus() === 'object'),
      error: computed(() => statusError(requestStatus())),
    })),
  );
}
