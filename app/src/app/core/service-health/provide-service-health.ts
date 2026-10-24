import { EnvironmentProviders, inject, Injector, makeEnvironmentProviders } from '@angular/core';
import { OPEN_SETTINGS, RAISE_CAP, ServiceHealthBackend, SERVICE_HEALTH_BACKEND, Unlisten } from '../../health/service-health.backend';
import { HealthSnapshot, RaiseCap } from '../../health/service-health.types';
import { isTauri } from '../platform';
import { TauriServiceHealthBackend } from './tauri-service-health.backend';

/** The mock, loaded only in the browser build. */
let mock: Promise<ServiceHealthBackend> | null = null;
const lazyMock = (): Promise<ServiceHealthBackend> => (mock ??= import('./mock-service-health.backend').then((m) => new m.MockServiceHealth()));

/** The browser build's backend: the shared mock, which end-to-end tests drive through `globalThis.__debriefMockHealth`. */
class LazyMockServiceHealth implements ServiceHealthBackend {
  async snapshot(): Promise<HealthSnapshot> {
    const m = await lazyMock();
    (globalThis as Record<string, unknown>)['__debriefMockHealth'] = m;
    return m.snapshot();
  }
  async repair(service: string): Promise<HealthSnapshot> {
    return (await lazyMock()).repair(service);
  }
  async onChange(handler: (s: HealthSnapshot) => void): Promise<Unlisten> {
    return (await lazyMock()).onChange(handler);
  }
}

/** Service health for the window: the Tauri commands in the desktop app, the mock in the browser build. */
export function provideServiceHealth(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: SERVICE_HEALTH_BACKEND, useFactory: () => (isTauri() ? new TauriServiceHealthBackend() : new LazyMockServiceHealth()) },
    {
      // "Raise the cap…" opens Settings ▸ Performance (or the plugin's cap) with the suggested value.
      provide: RAISE_CAP,
      useFactory: () => {
        const injector = inject(Injector);
        return async (cap: RaiseCap) => (await import('../../settings/performance/memory-actions')).raiseCapOfHealth(injector, cap);
      },
    },
    {
      // The `openSettings` repair (e.g. the research browser that could not start): Settings at the section that fixes it.
      provide: OPEN_SETTINGS,
      useFactory: () => {
        const injector = inject(Injector);
        return async (section: string) => injector.get((await import('../../settings/settings-dialog')).SettingsDialogService).open(section);
      },
    },
  ]);
}
