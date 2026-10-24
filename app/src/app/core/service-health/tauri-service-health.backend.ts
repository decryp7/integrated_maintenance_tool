import { invoke } from '../activity/ipc';
import { SERVICE_HEALTH_EVENT, HealthSnapshot } from '../../health/service-health.types';
import { ServiceHealthBackend, Unlisten } from '../../health/service-health.backend';
import { listenHere, PayloadListenFn } from '../../windowing/window-events';

const isSnapshot = (p: unknown): p is HealthSnapshot =>
  !!p && typeof p === 'object' && typeof (p as HealthSnapshot).revision === 'number' && Array.isArray((p as HealthSnapshot).services);

/** Service health through the Tauri commands `health_snapshot` and `health_repair` and the `debrief://service-health` event. */
export class TauriServiceHealthBackend implements ServiceHealthBackend {
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly listen: PayloadListenFn = listenHere,
  ) {}

  snapshot(): Promise<HealthSnapshot> {
    return this.call<HealthSnapshot>('health_snapshot');
  }

  repair(service: string): Promise<HealthSnapshot> {
    return this.call<HealthSnapshot>('health_repair', { service });
  }

  onChange(handler: (s: HealthSnapshot) => void): Promise<Unlisten> {
    return this.listen(SERVICE_HEALTH_EVENT, (p) => {
      if (isSnapshot(p)) handler(p);
    });
  }
}
