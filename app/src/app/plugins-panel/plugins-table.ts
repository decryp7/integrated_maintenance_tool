import { CdkMenu, CdkMenuItem, CdkMenuTrigger } from '@angular/cdk/menu';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { PluginHealthStore, PluginRow } from '../tools/health/plugin-health.store';
import { Icon } from '../shared/icon';
import { PluginMemoryStore } from '../settings/performance/plugin-memory.store';
import { InstalledPluginDto, PluginStatus } from '../core/api.types';
import { signatureText, stateLabel } from './package-labels';
import { PluginDetail } from './plugin-detail';
import { memoryView, provideGroups, ProvideGroup, statusLabel } from './plugin-model';
import { PluginPackagesStore } from './plugin-packages.store';

/** A package the engine is not running, as a table row (the engine reports only what it loaded). */
export function packageRow(k: InstalledPluginDto): PluginRow {
  const status: PluginStatus = k.state === 'quarantined' ? 'quarantined' : k.state === 'disabled' ? 'disabled' : k.state === 'running' ? 'active' : 'inactive';
  return {
    id: k.id,
    name: k.name,
    version: k.version,
    contributes: [],
    status,
    failures: 0,
    lastError: k.state === 'quarantined' ? k.problem : null,
    requires: [],
    provides: [],
    inactiveReason: k.state === 'unavailable' || k.state === 'pendingRestart' ? (k.problem ?? stateLabel(k.state)) : null,
    dependents: [],
    warnings: k.warning ? [k.warning] : [],
    hung: false,
    pendingEnabled: null,
  };
}

/** The installed plugins as one table (name, version, what they provide, memory, status); a row opens its side panel. */
@Component({
  selector: 'imt-plugins-table',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, CdkMenu, CdkMenuItem, CdkMenuTrigger, PluginDetail],
  template: `
    <div class="wrap">
      <table class="plugins" aria-label="Installed plugins" i18n-aria-label="@@pluginsPanel.installed.aria">
        <thead>
          <tr>
            <th scope="col" i18n="@@pluginsPanel.col.plugin">Plugin</th>
            <th scope="col" i18n="@@pluginsPanel.col.version">Version</th>
            <th scope="col" i18n="@@pluginsPanel.col.source">Source</th>
            <th scope="col" i18n="@@pluginsPanel.col.provides">Provides</th>
            <th scope="col" i18n="@@pluginsPanel.col.memory">Memory</th>
            <th scope="col" i18n="@@pluginsPanel.col.status">Status</th>
            <th scope="col" class="act"><span class="sr" i18n="@@pluginsPanel.col.actions">Actions</span></th>
          </tr>
        </thead>
        <tbody>
          @for (p of rows(); track p.id) {
            <tr class="plugin" [class.sel]="selectedId() === p.id" [id]="'plugin-row-' + p.id" tabindex="0" [attr.aria-selected]="selectedId() === p.id" [attr.data-id]="p.id" (click)="select(p.id)" (keydown.enter)="select(p.id)">
              <td class="who">
                <strong class="name">{{ p.name }}</strong>
                <span class="muted pub">{{ publisher(p) }}</span>
              </td>
              <td class="ver">{{ p.version }}</td>
              <td class="src" data-testid="plugin-source">
                @if (pkgOf(p.id); as k) {
                  <span i18n="@@pluginsPanel.source.package">Package</span>
                  @if (k.signature.kind !== 'trusted') { <span class="sigwarn" data-testid="package-signature">{{ signature(k.signature) }}</span> }
                } @else {
                  <span class="muted" i18n="@@pluginsPanel.source.builtIn">Built in</span>
                }
              </td>
              <td>
                <span class="provides">
                  @for (g of groups(p); track g.kind) {
                    <span class="grp" data-testid="plugin-group" [title]="g.items.join('\n')" [attr.aria-label]="groupLabel(g)"><imt-icon [name]="g.icon" [size]="13" /><span class="n">{{ g.items.length }}</span></span>
                  } @empty {
                    <span class="muted">—</span>
                  }
                </span>
              </td>
              <td class="memory">
                @if (memory(p); as m) {
                  <span class="mem" [attr.data-level]="m.level">
                    <span class="mtxt">{{ m.text }}</span>
                    @if (m.percent !== null) { <span class="bar" aria-hidden="true"><span class="fill" [style.width.%]="m.percent"></span></span> }
                    @if (m.note) { <span class="note" [class.danger]="m.level === 'danger'">{{ m.note }}</span> }
                  </span>
                } @else {
                  <span class="muted">—</span>
                }
              </td>
              <td>
                <span class="status" [attr.data-status]="p.status" data-testid="plugin-status">
                  <span class="dot" aria-hidden="true"></span>
                  @if (p.status === 'active') { <span class="sr">{{ status(p) }}</span> } @else { <span>{{ status(p) }}</span> }
                </span>
              </td>
              <td class="act" (click)="$event.stopPropagation()" (keydown.enter)="$event.stopPropagation()">
                <button type="button" class="icon-btn more" [cdkMenuTriggerFor]="rowMenu" [cdkMenuTriggerData]="{ $implicit: p }" [attr.aria-label]="moreLabel(p.name)" [title]="moreLabel(p.name)" data-testid="plugin-more"><imt-icon name="more" [size]="14" /></button>
              </td>
            </tr>
          }
        </tbody>
      </table>
    </div>
    @if (selected(); as s) {
      <imt-plugin-detail [plugin]="s" (closed)="close()" data-testid="plugin-detail" />
    }

    <ng-template #rowMenu let-p>
      <div cdkMenu class="menu" [attr.aria-label]="p.name">
        @if (p.status === 'quarantined') {
          <button type="button" cdkMenuItem class="mi" data-testid="plugin-reset" (cdkMenuItemTriggered)="reset(p.id)" i18n="reset a quarantined plugin|@@pluginsPanel.menu.reset">Reset</button>
        }
        @if (isOn(p)) {
          <button type="button" cdkMenuItem class="mi" data-testid="plugin-disable" (cdkMenuItemTriggered)="setEnabled(p.id, false)" i18n="@@pluginsPanel.menu.disable">Disable</button>
        } @else {
          <button type="button" cdkMenuItem class="mi" data-testid="plugin-enable" (cdkMenuItemTriggered)="setEnabled(p.id, true)" i18n="@@pluginsPanel.menu.enable">Enable</button>
        }
        @if (isPackage(p.id)) {
          <hr class="sep" />
          <button type="button" cdkMenuItem class="mi danger" data-testid="plugin-remove" (cdkMenuItemTriggered)="packages.remove(p.id)" i18n="@@pluginsPanel.menu.remove">Remove…</button>
        }
      </div>
    </ng-template>
  `,
  styles: `
    :host { display: block; }
    table { width: 100%; border-collapse: collapse; font-size: var(--fs-md); }
    th { background: var(--bg-1); text-align: start; font-weight: 500; font-size: var(--fs-sm); color: var(--muted); padding: 6px 12px; border-bottom: 1px solid var(--border); white-space: nowrap; }
    td { padding: 0 12px; height: 48px; border-bottom: 1px solid var(--border); white-space: nowrap; vertical-align: middle; }
    tr.plugin { cursor: pointer; }
    tr.plugin:hover, tr.plugin.sel { background: var(--bg-active); }
    tr.plugin:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
    .who { display: flex; flex-direction: column; justify-content: center; gap: 1px; }
    .name { font-size: var(--fs-md); font-weight: 600; }
    .pub { font-size: var(--fs-xs); }
    .muted { color: var(--muted); }
    .src { line-height: 1.3; }
    .sigwarn { display: block; font-size: var(--fs-xs); color: var(--warn-text); }
    .ver { font-variant-numeric: tabular-nums; font-weight: 600; }
    .provides { display: inline-flex; gap: 10px; align-items: center; }
    .grp { display: inline-flex; align-items: center; gap: 4px; color: var(--text-2); }
    .n { font-variant-numeric: tabular-nums; font-size: var(--fs-sm); }
    .mem { display: inline-flex; flex-direction: column; gap: 3px; min-width: 120px; }
    .mtxt { font-variant-numeric: tabular-nums; font-size: var(--fs-sm); }
    .bar { display: block; height: 3px; border-radius: 2px; background: var(--border-2); overflow: hidden; }
    .fill { display: block; height: 100%; background: var(--muted); }
    .mem[data-level='warn'] .fill { background: var(--lvl-warn); }
    .mem[data-level='danger'] .fill { background: var(--lvl-error); }
    .note { font-size: var(--fs-xs); color: var(--warn-text); }
    .note.danger { color: var(--danger-text, var(--lvl-error)); }
    .status { display: inline-flex; align-items: center; gap: 6px; }
    .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--ok); }
    .status[data-status='disabled'] .dot { background: var(--faint); }
    .status[data-status='inactive'] .dot { background: var(--lvl-warn); }
    .status[data-status='quarantined'] .dot { background: var(--lvl-error); }
    .act { width: 40px; text-align: end; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
    .menu { min-width: 160px; display: flex; flex-direction: column; padding: 4px; border-radius: 8px; background: var(--bg-2); border: 1px solid var(--border-3); box-shadow: var(--shadow); }
    .menu:focus { outline: none; }
    .mi { display: flex; align-items: center; padding: 5px 10px; border: 0; border-radius: 5px; background: transparent; color: var(--text); cursor: pointer; text-align: start; font: inherit; }
    .mi:hover, .mi:focus { background: var(--bg-active); outline: none; }
    .mi:focus-visible { box-shadow: inset 0 0 0 1px var(--accent); }
    .mi.danger { color: var(--danger-text, var(--lvl-error)); }
    .sep { border: 0; border-top: 1px solid var(--border-2); margin: 4px 0; width: 100%; }
  `,
})
export class PluginsTable {
  protected readonly health = inject(PluginHealthStore);
  protected readonly packages = inject(PluginPackagesStore);
  private readonly memories = inject(PluginMemoryStore);
  /** What the engine runs, plus packages it is not running (disabled, unavailable, waiting for a restart): one table, one row each. */
  protected readonly rows = computed<PluginRow[]>(() => {
    const running = this.health.plugins();
    const known = new Set(running.map((p) => p.id));
    const rest = this.packages.plugins().filter((k) => !known.has(k.id)).map((k) => packageRow(k));
    return [...running, ...rest];
  });
  protected readonly signature = signatureText;
  protected readonly selectedId = signal<string | null>(null);
  protected readonly selected = computed(() => this.rows().find((p) => p.id === this.selectedId()) ?? null);
  protected readonly status = (p: PluginRow): string => (p.pendingEnabled === null ? statusLabel(p.status) : p.pendingEnabled ? $localize`:@@pluginsPanel.status.willEnable:Starts after restart` : $localize`:@@pluginsPanel.status.willDisable:Stops after restart`);

  protected groups(p: PluginRow): ProvideGroup[] {
    return provideGroups(p.contributes);
  }
  protected memory(p: PluginRow) {
    return memoryView(this.memories.processes().find((m) => m.process.kind === 'plugin' && m.process.pluginId === p.id));
  }
  /** Who ships a plugin: its package's publisher, or IMT. */
  protected publisher(p: PluginRow): string {
    return this.packages.plugins().find((x) => x.id === p.id)?.publisher ?? $localize`:@@pluginsPanel.publisher.builtIn:Built in`;
  }
  protected pkgOf(id: string): InstalledPluginDto | null {
    return this.packages.plugins().find((x) => x.id === id) ?? null;
  }
  protected isPackage(id: string): boolean {
    return this.packages.plugins().some((x) => x.id === id);
  }
  protected isOn(p: PluginRow): boolean {
    const k = this.pkgOf(p.id);
    if (k) return k.enabled;
    return p.pendingEnabled ?? p.status !== 'disabled';
  }
  /** Packages are enabled and reset through the package store (it owns their record); built-in plugins through the health store. */
  protected setEnabled(id: string, on: boolean): void {
    if (this.isPackage(id)) void this.packages.setEnabled(id, on);
    else void this.health.setEnabled(id, on);
  }
  protected reset(id: string): void {
    if (this.isPackage(id)) void this.packages.reset(id);
    else void this.health.reset(id);
  }
  protected groupLabel(g: ProvideGroup): string {
    return $localize`:@@pluginsPanel.groupAria:${g.items.length}:count: ${g.kind}:kind:`;
  }
  protected moreLabel(name: string): string {
    return $localize`:@@pluginsPanel.moreAria:Actions for ${name}:name:`;
  }

  protected select(id: string): void {
    this.selectedId.set(this.selectedId() === id ? null : id);
  }

  /** Closes the panel and puts focus back on the row that opened it. */
  protected close(): void {
    const id = this.selectedId();
    this.selectedId.set(null);
    if (id) queueMicrotask(() => document.getElementById('plugin-row-' + id)?.focus());
  }
}
