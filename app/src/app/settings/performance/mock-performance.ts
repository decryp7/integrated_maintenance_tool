import type { PerformanceBackend } from './performance-backend';
import { MAX_PLUGIN_MB, MIN_PLUGIN_MB, Preset, ProcessMemory, Proposal } from './performance.types';

/** Plausible answers for browser development and UI tests; the engine (`performance/suggest.rs`) is the reference. */
export class MockPerformanceBackend implements PerformanceBackend {
  private plugins: ProcessMemory[] = [
    { process: { kind: 'plugin', pluginId: 'net.decryptology.windows' }, name: 'Windows diagnostics', capMb: 1024, overrideMb: null, usedMb: 410, peakMb: 420, state: 'warned' },
    { process: { kind: 'plugin', pluginId: 'net.decryptology.registry' }, name: 'Windows registry', capMb: 512, overrideMb: 512, usedMb: null, peakMb: null, state: null },
  ];

  async suggest(): Promise<Proposal> {
    return {
      environment: { totalMemoryMb: 16_384, freeMemoryMb: 12_000, cores: 8, diskFreeMb: 200_000, diskKind: null, aiModelMb: 0 },
      rows: [
        { key: 'storeCapMb', label: 'Database worker memory cap', current: 2048, suggested: 3072, unit: 'MB' },
        { key: 'importBudgetMb', label: 'Memory for imports', current: 768, suggested: 1152, unit: 'MB' },
        { key: 'queryBudgetMb', label: 'Memory for queries', current: 300, suggested: 450, unit: 'MB' },
        { key: 'queryThreads', label: 'Query threads', current: 4, suggested: 3, unit: null },
      ],
      values: { storeCapMb: 3072, importBudgetMb: 1152, queryBudgetMb: 450, queryThreads: 3 },
    };
  }

  async preset(preset: Preset): Promise<Proposal> {
    if (preset === 'standard') return { environment: null, rows: [], values: {} };
    return {
      environment: null,
      rows: [
        { key: 'importBudgetMb', label: 'Memory for imports', current: 768, suggested: 384, unit: 'MB' },
        { key: 'queryBudgetMb', label: 'Memory for queries', current: 300, suggested: 236, unit: 'MB' },
        { key: 'returnMemoryAtOnce', label: 'Give freed memory back at once', current: false, suggested: true, unit: null },
      ],
      values: { importBudgetMb: 384, queryBudgetMb: 236, returnMemoryAtOnce: true },
    };
  }

  async memory(): Promise<ProcessMemory[]> {
    return [
      { process: { kind: 'storeWorker' }, name: null, capMb: 2048, overrideMb: null, usedMb: 600, peakMb: 700, state: 'ended' },
      { process: { kind: 'ai' }, name: null, capMb: 1024, overrideMb: null, usedMb: null, peakMb: null, state: null },
      ...this.plugins,
    ];
  }

  async setPluginMemory(pluginId: string, mb: number | null): Promise<ProcessMemory[]> {
    if (mb !== null && (mb < MIN_PLUGIN_MB || mb > MAX_PLUGIN_MB)) throw new Error(`a plugin's memory cap is ${MIN_PLUGIN_MB} to ${MAX_PLUGIN_MB} MB`);
    if (!this.plugins.some((p) => p.process.kind === 'plugin' && p.process.pluginId === pluginId)) throw new Error(`there is no plugin "${pluginId}"`);
    this.plugins = this.plugins.map((p) => (p.process.kind === 'plugin' && p.process.pluginId === pluginId ? { ...p, overrideMb: mb, capMb: mb ?? 1024 } : p));
    return this.memory();
  }
}
