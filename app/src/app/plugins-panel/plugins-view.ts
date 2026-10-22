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
  imports: [PluginsInstalled, PluginHealthView],
  template: `
    <div class="tabs" role="tablist" aria-label="Plugins" i18n-aria-label="@@pluginsPanel.tabs">
      @for (t of tabs; track t.id) {
        <button type="button" role="tab" class="tab" [id]="'plugins-tab-' + t.id" [attr.aria-selected]="state.tab() === t.id" [attr.aria-controls]="'plugins-panel-' + t.id" [attr.tabindex]="state.tab() === t.id ? 0 : -1" [attr.data-testid]="'plugins-tab-' + t.id" (click)="state.show(t.id)" (keydown)="onKey($event)">{{ t.label }}</button>
      }
    </div>
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
    .tabs { flex: none; display: flex; gap: 2px; padding: 6px 12px 0; border-bottom: 1px solid var(--border); }
    .tab { border: 0; border-bottom: 2px solid transparent; background: transparent; color: var(--muted); padding: 6px 12px; font: inherit; cursor: pointer; }
    .tab[aria-selected='true'] { color: var(--text); border-bottom-color: var(--accent); }
    .tab:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
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

  /** Arrow keys move between the tabs. */
  protected onKey(e: KeyboardEvent): void {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
    e.preventDefault();
    const next: PluginsTab = this.state.tab() === 'installed' ? 'health' : 'installed';
    this.state.show(next);
    queueMicrotask(() => document.getElementById('plugins-tab-' + next)?.focus());
  }
}
