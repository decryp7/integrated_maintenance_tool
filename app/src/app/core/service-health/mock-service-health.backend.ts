import { ServiceHealthBackend, Unlisten } from '../../health/service-health.backend';
import { HealthCause, HealthSnapshot, RepairAction, ServiceHealth, ServiceState } from '../../health/service-health.types';

/** The services of the engine (`health/hub.rs` CATALOG), all working. */
const CATALOG: readonly [string, string][] = [
  ['store', 'the log store'],
  ['ai', 'the AI worker'],
  ['plugins', 'plugins'],
  ['browser', 'the research browser'],
  ['downloads', 'downloads'],
];

/**
 * Service health for the browser build and tests: every service works until `fail` says otherwise, and `repair` brings it
 * back (as the engine's repair does). It mimics the contract; the engine is the reference.
 */
export class MockServiceHealth implements ServiceHealthBackend {
  private revision = 1;
  private services: ServiceHealth[] = CATALOG.map(([service, label]) => ({ service, label, state: 'ok', cause: null, repair: null }));
  private readonly handlers = new Set<(s: HealthSnapshot) => void>();
  /** What `repair` was asked, for tests. */
  readonly repairs: string[] = [];
  /** Whether a repair succeeds (tests set false to see it fail). */
  repairSucceeds = true;

  private current(): HealthSnapshot {
    return { revision: this.revision, services: this.services.map((s) => ({ ...s })) };
  }

  private set(service: string, patch: Partial<ServiceHealth>): void {
    this.services = this.services.map((s) => (s.service === service ? { ...s, ...patch } : s));
    this.revision++;
    const snap = this.current();
    for (const h of this.handlers) h(snap);
  }

  /** The service stops (as the engine's supervisor would report). Saying the same again changes nothing. */
  fail(service: string, cause: HealthCause, repair: RepairAction | null = 'restart', state: ServiceState = 'down'): void {
    const now = this.services.find((s) => s.service === service);
    if (!now || (now.state === state && JSON.stringify(now.cause) === JSON.stringify(cause) && now.repair === repair)) return;
    this.set(service, { state, cause, repair });
  }

  /** The service works again. */
  recover(service: string): void {
    const now = this.services.find((s) => s.service === service);
    if (now && now.state !== 'ok') this.set(service, { state: 'ok', cause: null, repair: null });
  }

  async snapshot(): Promise<HealthSnapshot> {
    return this.current();
  }

  async repair(service: string): Promise<HealthSnapshot> {
    this.repairs.push(service);
    if (this.repairSucceeds) this.recover(service);
    return this.current();
  }

  async onChange(handler: (s: HealthSnapshot) => void): Promise<Unlisten> {
    this.handlers.add(handler);
    return () => this.handlers.delete(handler);
  }
}
