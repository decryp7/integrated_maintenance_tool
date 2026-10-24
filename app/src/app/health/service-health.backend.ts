import { InjectionToken } from '@angular/core';
import { HealthSnapshot, RaiseCap } from './service-health.types';

/** Stops listening. */
export type Unlisten = () => void;

/**
 * Where service health comes from. The app provides one (Tauri commands and event, or a mock) for
 * `SERVICE_HEALTH_BACKEND`: `ServiceHealthStore` injects it, so the store, chip and panels do not know the host.
 */
export interface ServiceHealthBackend {
  /** Every service's state now. A window asks when it starts: a new window must not assume everything is fine. */
  snapshot(): Promise<HealthSnapshot>;
  /** Repairs `service` from the shell. Returns at once; the outcome arrives through `onChange`. */
  repair(service: string): Promise<HealthSnapshot>;
  onChange(handler: (s: HealthSnapshot) => void): Promise<Unlisten>;
}

/** Knows no services: what a window without a host (a unit test, a dialog that did not wire one) sees. */
export class NoServices implements ServiceHealthBackend {
  async snapshot(): Promise<HealthSnapshot> {
    return { revision: 1, services: [] };
  }
  async repair(): Promise<HealthSnapshot> {
    return this.snapshot();
  }
  async onChange(): Promise<Unlisten> {
    return () => undefined;
  }
}

/**
 * Raises the memory cap a process was stopped over (opens the setting with the suggested value). The app provides it; without
 * one the chip offers no "Raise the cap…" button.
 */
export const RAISE_CAP = new InjectionToken<(cap: RaiseCap) => void | Promise<void>>('RAISE_CAP');

/**
 * Opens Settings at a section (the `openSettings` repair). The app provides it; without one the chip offers no button for
 * that repair.
 */
export const OPEN_SETTINGS = new InjectionToken<(section: string) => void | Promise<void>>('OPEN_SETTINGS');

export const SERVICE_HEALTH_BACKEND = new InjectionToken<ServiceHealthBackend>('SERVICE_HEALTH_BACKEND', {
  providedIn: 'root',
  factory: () => new NoServices(),
});
