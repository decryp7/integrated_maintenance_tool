import { Injector } from '@angular/core';
import { RaiseCap } from '../../health/service-health.types';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { EngineSettingsStore } from '../engine-settings.store';
import { SettingsDialogService } from '../settings-dialog';
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

/** The process a health cause's `raiseCap.target` names (`storeWorker`, `ai`, `plugin:<id>`). */
export function processOfTarget(target: string): ProcessRef | null {
  if (target === 'storeWorker') return { kind: 'storeWorker' };
  if (target === 'ai') return { kind: 'ai' };
  return target.startsWith('plugin:') && target.length > 7 ? { kind: 'plugin', pluginId: target.slice(7) } : null;
}

/** "Raise the cap…" of a service health cause: opens the cap with the suggested value. */
export async function raiseCapOfHealth(injector: Injector, cap: RaiseCap): Promise<void> {
  const process = processOfTarget(cap.target);
  if (process) await raiseMemoryCap(injector, process, cap.suggestedMb);
}
