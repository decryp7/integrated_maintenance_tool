import { inject, Injectable } from '@angular/core';
import { TOOL_IDS } from '../core/tools.types';
import { DebriefCommand, DebriefView } from '../extensions/tokens';
import { CollectorDialogService } from './collector/collector-dialog.service';
import { PluginsTabs } from '../plugins-panel/plugins-tabs';
import { ToolCatalog } from './tool-catalog.store';

/** Answers whether a tool is listed by the engine (see `ToolCatalog.listed`). */
export interface ToolListing {
  listed(id: string): boolean;
}

/**
 * The tool panels. Same extension point as plugin views. Panels of plugin tools are offered only while their tool is
 * listed (not in safe mode, not with the tools plugin disabled).
 */
export function toolsViews(tools: ToolListing): DebriefView[] {
  return [
    { id: 'perfmon', category: 'Tools', title: $localize`:@@tools.view.perfmon:Performance Monitor`, icon: 'chart', defaultPosition: 'center', layout: { area: 'document', min: { width: 420, height: 240 }, pref: { width: 640, height: 400 }, priority: 55 }, component: () => import('./perfmon/perfmon-view').then((m) => m.PerfmonView), enabled: () => tools.listed(TOOL_IDS.perfmonTemplates) },
    { id: 'process-monitor', category: 'Tools', title: $localize`:@@tools.view.processMonitor:Process Monitor`, icon: 'metrics', defaultPosition: 'center', layout: { area: 'document', min: { width: 420, height: 240 }, pref: { width: 640, height: 400 }, priority: 55 }, component: () => import('./monitor/monitor-view').then((m) => m.MonitorView), enabled: () => tools.listed(TOOL_IDS.processMonitor) },
    { id: 'event-log-follower', category: 'Tools', title: $localize`:@@tools.view.eventLog:Event Log Follower`, icon: 'file', defaultPosition: 'center', layout: { area: 'document', min: { width: 420, height: 240 }, pref: { width: 640, height: 400 }, priority: 55 }, component: () => import('./eventlog/eventlog-view').then((m) => m.EventLogView), enabled: () => tools.listed(TOOL_IDS.eventLog) },
    { id: 'saved-queries', category: 'Analysis', title: $localize`:@@tools.view.savedQueries:Saved queries`, icon: 'file', defaultPosition: 'right', layout: { area: 'side', min: { width: 320, height: 240 }, pref: { width: 372, height: 400 }, priority: 45 }, component: () => import('../projects/bookmarks/saved-queries-view').then((m) => m.SavedQueriesView) },
  ];
}

/** Commands that open the tools. */
@Injectable({ providedIn: 'root' })
export class ToolsCommands {
  private readonly collector = inject(CollectorDialogService);
  private readonly catalog = inject(ToolCatalog);
  private readonly plugins = inject(PluginsTabs);

  readonly commands: DebriefCommand[] = [
    { id: 'tools.perfmon.show', title: $localize`:@@tools.cmd.perfmon:Open Performance Monitor`, category: $localize`:@@tools.category.tools:Tools`, when: () => this.catalog.listed(TOOL_IDS.perfmonTemplates), run: (ctx) => ctx.showView('perfmon') },
    { id: 'tools.monitor.show', title: $localize`:@@tools.cmd.processMonitor:Open Process Monitor`, category: $localize`:@@tools.category.tools:Tools`, when: () => this.catalog.listed(TOOL_IDS.processMonitor), run: (ctx) => ctx.showView('process-monitor') },
    { id: 'tools.eventlog.show', title: $localize`:@@tools.cmd.eventLog:Open Event Log Follower`, category: $localize`:@@tools.category.tools:Tools`, when: () => this.catalog.listed(TOOL_IDS.eventLog), run: (ctx) => ctx.showView('event-log-follower') },
    {
      id: 'tools.health.show',
      title: $localize`:@@tools.cmd.pluginHealth:Open plugin health`,
      category: $localize`:@@tools.category.tools:Tools`,
      run: (ctx) => {
        this.plugins.show('health');
        ctx.showView('plugins');
      },
    },
    { id: 'tools.collect', title: $localize`:@@tools.cmd.collect:Collect diagnostics bundle…`, category: $localize`:@@tools.category.tools:Tools`, when: () => this.catalog.listed(TOOL_IDS.diagnostics), run: () => this.collector.open() },
  ];
}
