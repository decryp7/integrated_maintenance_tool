import { Tabs } from '../shared/tabs';
import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { PluginHealthView } from '../tools/health/plugin-health-view';
import { PluginsInstalled } from './plugins-installed';
import { PluginsTab, PluginsTabs } from './plugins-tabs';

export { BUILTIN_IMPORTERS, contributionLabel } from './plugins-installed';

/**
 * The one Plugins view (I4): *Installed* (what is installed, packages, memory, built-ins) and *Health* (status, repeat
 * failures, reset and enable/disable). Per-plugin settings are in Settings ▸ Plugins.
 */
@Component({
  selector: 'debrief-plugins-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [PluginsInstalled, PluginHealthView, Tabs],
  template: `
    <debrief-tabs class="tabs" label="Plugins" i18n-label="@@pluginsPanel.tabs" idPrefix="plugins-tab-" [items]="tabs" [selected]="state.tab()" [panelId]="panelOf" (selectedChange)="state.show($any($event))" />
    <div class="body" role="tabpanel" [id]="'plugins-panel-' + state.tab()" [attr.aria-labelledby]="'plugins-tab-' + state.tab()">
      @if (state.tab() === 'installed') {
        <debrief-plugins-installed />
      } @else {
        <debrief-plugin-health-view />
      }
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .tabs { flex: none; padding: 0 12px; border-bottom: 1px solid var(--border); }
    .body { flex: 1; min-height: 0; display: flex; flex-direction: column; }
    .body > * { flex: 1; min-height: 0; }
  `,
})
export class PluginsView {
  protected readonly state = inject(PluginsTabs);
  protected readonly tabs: readonly { id: PluginsTab; label: string }[] = [
    { id: 'installed', label: $localize`:@@pluginsPanel.tab.installed:Installed` },
    { id: 'health', label: $localize`:@@pluginsPanel.tab.health:Health` },
  ];

  protected readonly panelOf = (id: string) => 'plugins-panel-' + id;
}
