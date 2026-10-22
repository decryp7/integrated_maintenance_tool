import { Injector } from '@angular/core';
import { AiBackend } from '../../ai/ai-backend';
import { LifecycleBackend } from '../../core/tools-backend';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { EngineSettingsStore } from '../engine-settings.store';
import { SettingsDialogService } from '../settings-dialog';
import { PerformanceBackend } from './performance-backend';
import { PluginMemoryStore } from './plugin-memory.store';
import { PERFORMANCE_SECTION, ProcessRef } from './performance.types';

/** The id of the Plugins view, where a plugin's own cap is set. */
export const PLUGINS_VIEW = 'plugins';

/** The Settings ▸ Performance field holding the cap of `process` (plugins have their own cap instead). */
export function capField(process: ProcessRef): string | null {
  switch (process.kind) {
    case 'storeWorker':
      return 'storeCapMb';
    case 'ai':
      return 'aiWorkerMemoryMb';
    case 'plugin':
      return null;
  }
}

/**
 * "Raise the cap…" of a memory message: opens the cap with the suggested value filled in, not applied — Settings ▸
 * Performance for the database and AI workers, the plugin's own cap in the Plugins view for a plugin.
 */
export async function raiseMemoryCap(injector: Injector, process: ProcessRef, suggestedMb: number): Promise<void> {
  if (process.kind === 'plugin') {
    injector.get(PluginMemoryStore).request(process.pluginId, suggestedMb);
    injector.get(COMMAND_CONTEXT).showView(PLUGINS_VIEW);
    return;
  }
  const settings = injector.get(EngineSettingsStore);
  injector.get(SettingsDialogService).open(PERFORMANCE_SECTION);
  await settings.load();
  const key = capField(process);
  if (key) settings.edit(PERFORMANCE_SECTION, key, suggestedMb);
}

/** "Retry" of a memory message: lets the stopped process start again (its next use starts it). */
export async function retryProcess(injector: Injector, process: ProcessRef): Promise<void> {
  switch (process.kind) {
    case 'storeWorker':
      return injector.get(PerformanceBackend).retryStoreWorker();
    case 'ai':
      await injector.get(AiBackend).reset();
      return;
    case 'plugin':
      return injector.get(LifecycleBackend).pluginReset(process.pluginId);
  }
}
