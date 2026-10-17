import { inject, Injectable } from '@angular/core';
import { TOOL_IDS } from '../core/tools.types';
import { ImtCommand, ImtView } from '../extensions/tokens';
import { CollectorDialogService } from './collector/collector-dialog';
import { ToolCatalog } from './tool-catalog.store';

/** Answers whether a tool is listed by the engine (see `ToolCatalog.listed`). */
export interface ToolListing {
  listed(id: string): boolean;
}

/**
 * The tool panels. Same extension point as plugin views. Panels of plugin tools are offered only while their tool is
 * listed (not in safe mode, not with the tools plugin disabled).
 */
export function toolsViews(tools: ToolListing): ImtView[] {
  return [
    { id: 'perfmon', category: 'Tools', title: 'Performance Monitor', icon: 'chart', defaultPosition: 'center', component: () => import('./perfmon/perfmon-view').then((m) => m.PerfmonView), enabled: () => tools.listed(TOOL_IDS.perfmonTemplates) },
    { id: 'process-monitor', category: 'Tools', title: 'Process Monitor', icon: 'metrics', defaultPosition: 'center', component: () => import('./monitor/monitor-view').then((m) => m.MonitorView), enabled: () => tools.listed(TOOL_IDS.processMonitor) },
    { id: 'plugin-health', category: 'Diagnostics', title: 'Plugin health', icon: 'puzzle', defaultPosition: 'center', component: () => import('./health/plugin-health-view').then((m) => m.PluginHealthView) },
    { id: 'bookmarks', category: 'Analysis', title: 'Bookmarks', icon: 'file', defaultPosition: 'right', component: () => import('../projects/bookmarks/bookmarks-view').then((m) => m.BookmarksView) },
  ];
}

/** Commands that open the tools. */
@Injectable({ providedIn: 'root' })
export class ToolsCommands {
  private readonly collector = inject(CollectorDialogService);
  private readonly catalog = inject(ToolCatalog);

  readonly commands: ImtCommand[] = [
    { id: 'tools.perfmon.show', title: 'Open Performance Monitor', category: 'Tools', when: () => this.catalog.listed(TOOL_IDS.perfmonTemplates), run: (ctx) => ctx.showView('perfmon') },
    { id: 'tools.monitor.show', title: 'Open Process Monitor', category: 'Tools', when: () => this.catalog.listed(TOOL_IDS.processMonitor), run: (ctx) => ctx.showView('process-monitor') },
    { id: 'tools.health.show', title: 'Open plugin health', category: 'Tools', run: (ctx) => ctx.showView('plugin-health') },
    { id: 'tools.collect', title: 'Collect diagnostics bundle…', category: 'Tools', when: () => this.catalog.listed(TOOL_IDS.diagnostics), run: () => this.collector.open() },
  ];
}
