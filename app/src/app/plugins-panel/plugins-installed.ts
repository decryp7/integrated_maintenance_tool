import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { PluginsTable } from './plugins-table';
import { PluginHealthStore } from '../tools/health/plugin-health.store';
import { ToolError } from '../tools/elevation-hint';
import { Icon } from '../shared/icon';
import { PluginMemoryStore } from '../settings/performance/plugin-memory.store';
import { ExporterInfo, PluginInfo } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { InstalledPackages } from './installed-packages';

/** Importers that ship with IMT (plugins can add more). */
export const BUILTIN_IMPORTERS = [
  { id: 'json', name: $localize`:@@pluginsPanel.importer.json:JSON lines / CLEF (Serilog)` },
  { id: 'text', name: $localize`:@@pluginsPanel.importer.text:Plain text logs (output templates and regexes)` },
  { id: 'csv', name: 'CSV' },
];

/** Contribution kinds shown as badges, in display order; unknown kinds are shown as written. */
export function contributionLabel(contribution: string): string {
  return contribution.startsWith('transform:') ? $localize`:@@pluginsPanel.contribution.transform:transform` : contribution;
}

/** The Installed tab of the Plugins view: backend plugins reported by the engine, third-party packages, plus what is built in. */
@Component({
  selector: 'imt-plugins-installed',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [InstalledPackages, PluginsTable, ToolError, Icon],
  template: `
    <div class="scroll" tabindex="0" role="region" aria-label="Plugins overview" i18n-aria-label="@@pluginsPanel.region">
      <h2 class="section-title" i18n="@@pluginsPanel.installed">Installed plugins</h2>
      <imt-tool-error [message]="health.error()" (dismissed)="health.dismissError()" />
      @if (health.needsRestart()) {
        <div class="restart" role="status">
          <span i18n="@@pluginsPanel.restartNote">Plugin changes apply after a restart.</span>
          <button type="button" class="btn primary" (click)="health.restart()" i18n="@@pluginsPanel.restartApply">Restart to apply</button>
        </div>
      }
      @if (plugins().length) {
        <imt-plugins-table />
      } @else {
        <div class="empty" data-testid="plugins-empty">
          <imt-icon name="puzzle" [size]="32" />
          <h3 i18n="@@pluginsPanel.empty.title">No plugins installed</h3>
          <p i18n="@@pluginsPanel.none">IMT works with its built-in importers and exporters. Install a plugin from a file to add more.</p>
        </div>
      }

      <imt-installed-packages />

      <h2 class="section-title" i18n="@@pluginsPanel.builtIn">Built in</h2>
      <div class="builtin">
        <div><span class="faint" i18n="@@pluginsPanel.importers">Importers</span>
          <div class="badges">@for (i of importers; track i.id) { <span class="badge" [title]="i.name">{{ i.id }}</span> }</div>
        </div>
        <div><span class="faint" i18n="@@pluginsPanel.exporters">Exporters</span>
          <div class="badges">@for (e of exporters(); track e.id) { <span class="badge" [title]="e.name">{{ e.id }}</span> } @empty { <span class="faint" i18n="@@pluginsPanel.noExporters">none</span> }</div>
        </div>
      </div>

      <h2 class="section-title" i18n="@@pluginsPanel.about">About plugins</h2>
      <div class="help">
        <p i18n="@@pluginsPanel.about.p1">A plugin extends IMT without changing the app. Backend plugins can add <strong>importers</strong> (read a new file format), <strong>transforms</strong> (a step you can use in the profile pipeline), <strong>exporters</strong> (write events to a new format) and <strong>hooks</strong> (react to events such as a finished import).</p>
        <p i18n="@@pluginsPanel.about.p2">Plugins are loaded by the engine when it starts; what it found is listed above. A plugin's importer shows up in the Explorer as the format badge of its files, and a transform can be added from the Pipeline section of the profile.</p>
        <p i18n="@@pluginsPanel.about.p3">UI plugins add panels (like the Registry viewer), commands, inspector sections and field formatters, and are registered when the app is built.</p>
      </div>
    </div>
  `,
  styles: `
    :host { position: relative; flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .scroll { flex: 1; min-height: 0; overflow: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 10px; }
    .scroll:focus-visible { outline-offset: -2px; }
    h2 { margin: 8px 0 0; }
    .restart { display: flex; align-items: center; gap: 12px; padding: 8px 10px; border: 1px solid var(--accent); border-radius: 6px; background: var(--bg-3); }
    .empty { display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 24px 0; color: var(--muted); text-align: center; }
    .empty h3 { margin: 0; font-size: var(--fs-lg); font-weight: 600; color: var(--text); }
    .empty p { margin: 0; max-width: 360px; }
    .builtin { display: flex; gap: 32px; flex-wrap: wrap; }
    .help { max-width: 640px; color: var(--text-2); font-size: var(--fs-md); line-height: 1.5; }
    .help p { margin: 0 0 8px; }
  `,
})
export class PluginsInstalled {
  private readonly store = inject(WorkspaceStore);
  private readonly memory = inject(PluginMemoryStore);
  protected readonly health = inject(PluginHealthStore);

  constructor() {
    void this.memory.load();
  }

  protected readonly importers = BUILTIN_IMPORTERS;
  protected readonly plugins = computed<PluginInfo[]>(() => this.store.summary()?.plugins ?? []);
  protected readonly exporters = computed<ExporterInfo[]>(() => this.store.summary()?.exporters ?? []);
}
