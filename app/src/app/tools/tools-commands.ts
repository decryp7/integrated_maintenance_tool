import { inject, Injectable } from '@angular/core';
import { ImtCommand, ImtView } from '../extensions/tokens';
import { CollectorDialogService } from './collector/collector-dialog';

/** The tool panels. Same extension point as plugin views. */
export const TOOLS_VIEWS: ImtView[] = [
  { id: 'perfmon', title: 'Performance Monitor', icon: 'chart', defaultPosition: 'center', component: () => import('./perfmon/perfmon-view').then((m) => m.PerfmonView) },
  { id: 'process-monitor', title: 'Process Monitor', icon: 'metrics', defaultPosition: 'center', component: () => import('./monitor/monitor-view').then((m) => m.MonitorView) },
  { id: 'plugin-health', title: 'Plugin health', icon: 'puzzle', defaultPosition: 'center', component: () => import('./health/plugin-health-view').then((m) => m.PluginHealthView) },
  { id: 'bookmarks', title: 'Bookmarks', icon: 'file', defaultPosition: 'right', component: () => import('../projects/bookmarks/bookmarks-view').then((m) => m.BookmarksView) },
];

/** Commands that open the tools. */
@Injectable({ providedIn: 'root' })
export class ToolsCommands {
  private readonly collector = inject(CollectorDialogService);

  readonly commands: ImtCommand[] = [
    { id: 'tools.perfmon.show', title: 'Open Performance Monitor', category: 'Tools', run: (ctx) => ctx.showView('perfmon') },
    { id: 'tools.monitor.show', title: 'Open Process Monitor', category: 'Tools', run: (ctx) => ctx.showView('process-monitor') },
    { id: 'tools.health.show', title: 'Open plugin health', category: 'Tools', run: (ctx) => ctx.showView('plugin-health') },
    { id: 'tools.collect', title: 'Collect diagnostics bundle…', category: 'Tools', run: () => this.collector.open() },
  ];
}
