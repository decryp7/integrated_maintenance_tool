import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withProps, withState } from '@ngrx/signals';
import { setError, setLoaded, setLoading, withRequestStatus } from '../shared/store/with-request-status';
import { SERVICE_HEALTH_BACKEND } from './service-health.backend';
import { EMPTY_HEALTH, HealthSnapshot, isServing, ServiceHealth } from './service-health.types';

interface HealthState {
  snapshot: HealthSnapshot;
  /** Set by "Waiting for ..." links: the status chip opens its details. */
  detailsRequested: number;
}

/**
 * The latest service-health snapshot of this window and nothing else (ADR 0055): the engine decides, an event
 * announces, this store holds the newest. Nothing is derived from error text or from which commands succeeded. Until
 * the first snapshot arrived the window does not claim anything is healthy (`isReady` is false and `isServing` is false).
 */
export const ServiceHealthStore = signalStore(
  { providedIn: 'root', protectedState: true },
  withState<HealthState>({ snapshot: EMPTY_HEALTH, detailsRequested: 0 }),
  withRequestStatus(),
  withComputed((store) => ({
    services: computed(() => store.snapshot().services),
    /** The services that are not working (any state but ok and starting), in the registry's order. */
    problems: computed(() => store.snapshot().services.filter((s) => !isServing(s.state))),
    /** The first snapshot arrived. */
    isReady: computed(() => store.snapshot().revision > 0 || store.requestStatus() === 'loaded'),
  })),
  withProps(() => ({ backend: inject(SERVICE_HEALTH_BACKEND) })),
  withMethods((store) => {
    // Events can cross: a window keeps the newest.
    const apply = (next: HealthSnapshot): void => patchState(store, (s) => (next.revision >= s.snapshot.revision ? { snapshot: next } : s));
    return {
      /** The service now, or null while the window has not heard (or the service is unknown). */
      health(service: string): ServiceHealth | null {
        return store.services().find((s) => s.service === service) ?? null;
      },
      /** Whether things that need `service` can be asked of it. Before the first snapshot: not. */
      isServing(service: string): boolean {
        // Before the first snapshot nothing is known, so nothing is claimed healthy; afterwards a service the engine does
        // not list has not failed.
        if (!store.isReady()) return false;
        const h = store.services().find((s) => s.service === service);
        return !h || isServing(h.state);
      },
      apply,
      /** Asks the engine for every service's state (when the window starts, and to try again). */
      async load(): Promise<void> {
        patchState(store, setLoading());
        try {
          const next = await store.backend.snapshot();
          apply(next);
          patchState(store, setLoaded());
        } catch (e) {
          patchState(store, setError(e instanceof Error ? e.message : String(e)));
        }
      },
      /** The Repair / Try again button: the shell repairs the service; its state follows by event. */
      async repair(service: string): Promise<void> {
        try {
          apply(await store.backend.repair(service));
        } catch (e) {
          patchState(store, setError(e instanceof Error ? e.message : String(e)));
        }
      },
      /** Opens the status chip's details (the "Waiting for ..." link). */
      showDetails(): void {
        patchState(store, (s) => ({ detailsRequested: s.detailsRequested + 1 }));
      },
    };
  }),
  withHooks((store) => {
    let off: (() => void) | null = null;
    let closed = false;
    return {
      onInit() {
        void store.backend.onChange((s) => store.apply(s)).then((u) => {
          if (closed) u();
          else off = u;
        });
        void store.load();
      },
      onDestroy() {
        closed = true;
        off?.();
      },
    };
  }),
);
