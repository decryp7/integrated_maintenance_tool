import { inject, Injectable } from '@angular/core';
import { invoke } from '../../core/activity/ipc';
import { reportBackendFailures } from '../../core/failure-reporter';
import { Logger, withFailureLogging } from '../../core/logger';
import { isTauri } from '../../core/platform';
import { Preset, ProcessMemory, Proposal } from './performance.types';

/**
 * Settings ▸ Performance's own commands (contract: `performance.types.ts`): proposals (Suggest settings, presets), the
 * memory of the capped processes, plugin caps and letting the store worker start again. Suggestions and validation
 * are the engine's; the UI shows and applies them.
 */
@Injectable({ providedIn: 'root', useFactory: () => createPerformanceBackend() })
export abstract class PerformanceBackend {
  abstract suggest(): Promise<Proposal>;
  abstract preset(preset: Preset): Promise<Proposal>;
  abstract memory(): Promise<ProcessMemory[]>;
  /** Sets (`mb`) or removes (`null`) a plugin's own cap; answers the processes with their caps now. */
  abstract setPluginMemory(pluginId: string, mb: number | null): Promise<ProcessMemory[]>;
  abstract retryStoreWorker(): Promise<void>;
}

export class TauriPerformanceBackend implements PerformanceBackend {
  constructor(private readonly call: typeof invoke = invoke) {}
  suggest() {
    return this.call<Proposal>('performance_suggest');
  }
  preset(preset: Preset) {
    return this.call<Proposal>('performance_preset', { preset });
  }
  memory() {
    return this.call<ProcessMemory[]>('performance_memory');
  }
  setPluginMemory(pluginId: string, mb: number | null) {
    return this.call<ProcessMemory[]>('performance_set_plugin_memory', { pluginId, mb });
  }
  retryStoreWorker() {
    return this.call<void>('performance_retry_store_worker');
  }
}

export function createPerformanceBackend(): PerformanceBackend {
  const target: PerformanceBackend = isTauri() ? new TauriPerformanceBackend() : lazyMock();
  return withFailureLogging<PerformanceBackend>(target, inject(Logger), { onFailure: reportBackendFailures() });
}

function lazyMock(): PerformanceBackend {
  let loaded: Promise<PerformanceBackend> | null = null;
  const get = () => (loaded ??= import('./mock-performance').then((m) => new m.MockPerformanceBackend()));
  return {
    suggest: () => get().then((b) => b.suggest()),
    preset: (p) => get().then((b) => b.preset(p)),
    memory: () => get().then((b) => b.memory()),
    setPluginMemory: (id, mb) => get().then((b) => b.setPluginMemory(id, mb)),
    retryStoreWorker: () => get().then((b) => b.retryStoreWorker()),
  };
}
