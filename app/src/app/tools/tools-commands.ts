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
    { id: 'perfmon', category: 'Tools', title: $localize`:@@tools.view.perfmon:Performance Monitor`, icon: 'chart', defaultPosition: 'center', component: () => import('./perfmon/perfmon-view').then((m) => m.PerfmonView), enabled: () => tools.listed(TOOL_IDS.perfmonTemplates) },
    { id: 'process-monitor', category: 'Tools', title: $localize`:@@tools.view.processMonitor:Process Monitor`, icon: 'metrics', defaultPosition: 'center', component: () => import('./monitor/monitor-view').then((m) => m.MonitorView), enabled: () => tools.listed(TOOL_IDS.processMonitor) },
    { id: 'event-log-follower', category: 'Tools', title: $localize`:@@tools.view.eventLog:Event Log Follower`, icon: 'file', defaultPosition: 'center', component: () => import('./eventlog/eventlog-view').then((m) => m.EventLogView), enabled: () => tools.listed(TOOL_IDS.eventLog) },
    { id: 'plugin-health', category: 'Diagnostics', title: $localize`:@@tools.view.pluginHealth:Plugin health`, icon: 'puzzle', defaultPosition: 'center', component: () => import('./health/plugin-health-view').then((m) => m.PluginHealthView) },
    { id: 'bookmarks', category: 'Analysis', title: $localize`:@@tools.view.bookmarks:Bookmarks`, icon: 'file', defaultPosition: 'right', component: () => import('../projects/bookmarks/bookmarks-view').then((m) => m.BookmarksView) },
  ];
}

/** Commands that open the tools. */
@Injectable({ providedIn: 'root' })
export class ToolsCommands {
  private readonly collector = inject(CollectorDialogService);
  private readonly catalog = inject(ToolCatalog);

  readonly commands: ImtCommand[] = [
    { id: 'tools.perfmon.show', title: $localize`:@@tools.cmd.perfmon:Open Performance Monitor`, category: $localize`:@@tools.category.tools:Tools`, when: () => this.catalog.listed(TOOL_IDS.perfmonTemplates), run: (ctx) => ctx.showView('perfmon') },
    { id: 'tools.monitor.show', title: $localize`:@@tools.cmd.processMonitor:Open Process Monitor`, category: $localize`:@@tools.category.tools:Tools`, when: () => this.catalog.listed(TOOL_IDS.processMonitor), run: (ctx) => ctx.showView('process-monitor') },
    { id: 'tools.eventlog.show', title: $localize`:@@tools.cmd.eventLog:Open Event Log Follower`, category: $localize`:@@tools.category.tools:Tools`, when: () => this.catalog.listed(TOOL_IDS.eventLog), run: (ctx) => ctx.showView('event-log-follower') },
    { id: 'tools.health.show', title: $localize`:@@tools.cmd.pluginHealth:Open plugin health`, category: $localize`:@@tools.category.tools:Tools`, run: (ctx) => ctx.showView('plugin-health') },
    { id: 'tools.collect', title: $localize`:@@tools.cmd.collect:Collect diagnostics bundle…`, category: $localize`:@@tools.category.tools:Tools`, when: () => this.catalog.listed(TOOL_IDS.diagnostics), run: () => this.collector.open() },
  ];
}
