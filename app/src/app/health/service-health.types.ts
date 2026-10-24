// Wire contract of service health (`health_snapshot`, `health_repair` and the `debrief://service-health` event;
// camelCase JSON), mirrored by app/src-tauri/src/health/model.rs. Change both sides together.
//
// Shared-ready (Paddock rule 39, ADR 0010): everything in this folder knows nothing about Debrief's domain. It moves
// into `@decryp7/paddock-ui` unchanged once a second app needs it; the adapters (`core/service-health/`) stay behind.

export type ServiceState = 'ok' | 'starting' | 'degraded' | 'down' | 'quarantined' | 'repairing';

/** What the shell can do to bring a service back; it never goes through the service itself. */
/** `openSettings` is the UI's own: it opens the Settings section the cause names (`HealthCause.settings`). */
export type RepairAction = 'restart' | 'reset' | 'liftQuarantine' | 'install' | 'retry' | 'openSettings';

/** A memory cap stopped the process: the user can raise it (the UI opens the setting with this value). */
export interface RaiseCap {
  /** Which process: `storeWorker`, `ai` or `plugin:<id>`. */
  target: string;
  suggestedMb: number;
}

export interface HealthCause {
  /** One plain sentence. */
  message: string;
  /** The next step for this cause. */
  hint: string | null;
  /** The technical chain, for Details and bug reports. */
  detail: string | null;
  /** Set when a memory cap stopped the process: "Raise the cap…" is offered next to the repair. */
  raiseCap: RaiseCap | null;
  /** With the `openSettings` repair: the Settings section that fixes it (e.g. `browser`, Settings ▸ Web browser). */
  settings?: string | null;
}

export interface ServiceHealth {
  /** Stable id (`store`, `ai`, ...). */
  service: string;
  /** What the user calls it ("the log store"). */
  label: string;
  state: ServiceState;
  cause: HealthCause | null;
  repair: RepairAction | null;
}

export interface HealthSnapshot {
  /** Grows with every change; a window keeps the newest. */
  revision: number;
  services: ServiceHealth[];
}

/** The event every window listens to. */
export const SERVICE_HEALTH_EVENT = 'debrief://service-health';

/** A snapshot with nothing known: nothing is claimed healthy either (see `ServiceHealthStore.isReady`). */
export const EMPTY_HEALTH: HealthSnapshot = { revision: 0, services: [] };

/** The notification cause of a service outage (one notification per service). */
export function healthCause(service: string): string {
  return `health:${service}`;
}

/** Whether `cause` is a service outage that the status chip already states. */
export function isHealthCause(cause: string): boolean {
  return cause.startsWith('health:');
}

/** Whether things that need the service can be asked of it now. */
export function isServing(state: ServiceState): boolean {
  return state === 'ok' || state === 'starting';
}

/** The label of the repair button for an action. */
export function repairLabel(action: RepairAction): string {
  switch (action) {
    case 'restart':
      return $localize`:@@health.repair.restart:Restart`;
    case 'liftQuarantine':
      return $localize`:@@health.repair.liftQuarantine:Allow again`;
    case 'install':
      return $localize`:@@health.repair.install:Install`;
    case 'openSettings':
      return $localize`:@@health.repair.openSettings:Open Settings…`;
    case 'reset':
    case 'retry':
      return $localize`:@@health.repair.tryAgain:Try again`;
  }
}
