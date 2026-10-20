import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { ExporterInfo, PluginInfo } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';

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

/** What is installed: backend plugins reported by the engine, plus what is built in. */
@Component({
  selector: 'imt-plugins-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="scroll" tabindex="0" role="region" aria-label="Plugins overview" i18n-aria-label="@@pluginsPanel.region">
      <h2 class="section-title" i18n="@@pluginsPanel.installed">Installed plugins</h2>
      @if (plugins().length) {
        <ul class="list" aria-label="Installed plugins" i18n-aria-label="@@pluginsPanel.installed.aria">
          @for (p of plugins(); track p.id) {
            <li class="plugin">
              <div class="top"><strong>{{ p.name }}</strong> <span class="mono faint">v{{ p.version }}</span> <span class="mono faint id">{{ p.id }}</span></div>
              <div class="badges">
                @for (c of p.contributes; track c) { <span class="badge" [title]="c">{{ label(c) }}</span> }
              </div>
            </li>
          }
        </ul>
      } @else {
        <p class="faint" i18n="@@pluginsPanel.none">No plugins are installed. IMT works with its built-in importers and exporters.</p>
      }

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
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .scroll { flex: 1; min-height: 0; overflow: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 10px; }
    .scroll:focus-visible { outline-offset: -2px; }
    h2 { margin: 8px 0 0; }
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
    .plugin { padding: 8px 10px; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); display: flex; flex-direction: column; gap: 6px; }
    .top { display: flex; gap: 8px; align-items: baseline; }
    .id { margin-left: auto; font-size: var(--fs-xs); }
    .badges { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 4px; }
    .builtin { display: flex; gap: 32px; flex-wrap: wrap; }
    .help { max-width: 640px; color: var(--text-2); font-size: var(--fs-md); line-height: 1.5; }
    .help p { margin: 0 0 8px; }
  `,
})
export class PluginsView {
  private readonly store = inject(WorkspaceStore);
  protected readonly importers = BUILTIN_IMPORTERS;
  protected readonly label = contributionLabel;
  protected readonly plugins = computed<PluginInfo[]>(() => this.store.summary()?.plugins ?? []);
  protected readonly exporters = computed<ExporterInfo[]>(() => this.store.summary()?.exporters ?? []);
}
