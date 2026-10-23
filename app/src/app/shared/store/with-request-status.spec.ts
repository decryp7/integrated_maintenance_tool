import { TestBed } from '@angular/core/testing';
import { patchState, signalStore, withMethods, withState } from '@ngrx/signals';
import { setError, setIdle, setLoaded, setLoading, statusError, withRequestStatus } from './with-request-status';

const Store = signalStore(
  { providedIn: 'root' },
  withState({ data: null as string | null }),
  withRequestStatus(),
  withMethods((store) => ({
    async load(fetch: () => Promise<string>): Promise<void> {
      patchState(store, setLoading());
      try {
        patchState(store, { data: await fetch() }, setLoaded());
      } catch (e) {
        patchState(store, setError(String(e)));
      }
    },
    reset: () => patchState(store, setIdle()),
  })),
);

describe('withRequestStatus', () => {
  it('starts idle with no error', () => {
    const s = TestBed.inject(Store);
    expect(s.requestStatus()).toBe('idle');
    expect([s.isIdle(), s.isLoading(), s.isLoaded(), s.isError(), s.error()]).toEqual([true, false, false, false, null]);
  });

  it('is loading while the request runs, then loaded with the data', async () => {
    const s = TestBed.inject(Store);
    let release!: (v: string) => void;
    const pending = s.load(() => new Promise<string>((r) => (release = r)));
    expect(s.isLoading()).toBe(true);
    expect(s.isLoaded()).toBe(false);
    release('rows');
    await pending;
    expect([s.isLoading(), s.isLoaded(), s.data()]).toEqual([false, true, 'rows']);
  });

  it('carries the message on failure and clears it on the next request', async () => {
    const s = TestBed.inject(Store);
    await s.load(() => Promise.reject('disk gone'));
    expect([s.isError(), s.isLoading(), s.error()]).toEqual([true, false, 'disk gone']);
    const next = s.load(() => Promise.resolve('ok'));
    expect(s.error()).toBeNull();
    await next;
    expect(s.isLoaded()).toBe(true);
  });

  it('goes back to idle', async () => {
    const s = TestBed.inject(Store);
    await s.load(() => Promise.resolve('x'));
    s.reset();
    expect(s.isIdle()).toBe(true);
  });

  it('statusError reads only the error variant', () => {
    expect(statusError('loading')).toBeNull();
    expect(statusError({ error: 'm' })).toBe('m');
  });
});
