import { ChangeDetectionStrategy, Component, computed, inject, input, output } from '@angular/core';
import { PluginRow } from '../tools/health/plugin-health.store';
import { Icon } from '../shared/icon';
import { DialogService } from '../ui/dialog.service';
import { permissionTitle, signatureText, stateLabel } from './package-labels';
import { PluginMemory } from './plugin-memory';
import { PluginPackagesStore } from './plugin-packages.store';
import { PluginPermissionsDialog } from './plugin-permissions-dialog';
import { provideGroups, statusLabel } from './plugin-model';

/** The side panel of a plugin: everything it provides (identifiers), its memory cap and its permissions. */
@Component({
  selector: 'debrief-plugin-detail',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, PluginMemory],
  host: { '(keydown.escape)': 'closed.emit()' },
  template: `
    <header class="head">
      <div class="titles">
        <h3 class="name">{{ plugin().name }}</h3>
        <span class="muted sub"><span class="ver">{{ plugin().version }}</span> · {{ status() }}</span>
      </div>
      <button type="button" class="icon-btn" aria-label="Close details" i18n-aria-label="@@pluginsPanel.detail.close" title="Close" i18n-title="@@pluginsPanel.detail.closeTitle" (click)="closed.emit()" data-testid="plugin-detail-close"><debrief-icon name="close" [size]="14" /></button>
    </header>
    <div class="body">
      <section>
        <h4 i18n="@@pluginsPanel.detail.id">Plugin id</h4>
        <code class="mono id">{{ plugin().id }}</code>
      </section>
      @if (pkg(); as k) {
        <section data-testid="plugin-package">
          <h4 i18n="@@pluginsPanel.detail.source">Source</h4>
          <p>{{ k.publisher }}</p>
          <p class="muted" data-testid="package-signature">{{ signature(k.signature) }}</p>
          <p class="muted" data-testid="package-state">{{ state(k.state) }}</p>
          @if (k.problem) { <p class="danger" data-testid="package-problem">{{ k.problem }}</p> }
        </section>
      }
      @if (plugin().lastError || plugin().inactiveReason || plugin().warnings.length) {
        <section>
          <h4 i18n="@@pluginsPanel.detail.problems">Problems</h4>
          @if (plugin().inactiveReason) { <p class="warn">{{ plugin().inactiveReason }}</p> }
          @if (plugin().lastError) { <p class="danger">{{ plugin().lastError }}</p> }
          @for (w of plugin().warnings; track $index) { <p class="warn">{{ w }}</p> }
        </section>
      }
      <section>
        <h4 i18n="@@pluginsPanel.detail.provides">Provides</h4>
        <ul class="caps" aria-label="Capabilities" i18n-aria-label="@@pluginsPanel.detail.caps" data-testid="plugin-caps">
          @for (g of groups(); track g.kind) {
            @for (c of g.items; track c) {
              <li class="mono"><debrief-icon [name]="g.icon" [size]="12" />{{ c }}</li>
            }
          } @empty {
            <li class="muted" i18n="@@pluginsPanel.detail.noCaps">Nothing yet.</li>
          }
        </ul>
      </section>
      <section>
        <h4 i18n="@@pluginsPanel.detail.memory">Memory</h4>
        <debrief-plugin-memory [pluginId]="plugin().id" [pluginName]="plugin().name" />
      </section>
      <section data-testid="plugin-permissions">
        <h4 i18n="@@pluginsPanel.detail.permissions">Permissions</h4>
        @if (pkg(); as p) {
          @if (p.granted.length) {
            <ul class="perms">@for (g of p.granted; track g) { <li>{{ title(g) }}</li> }</ul>
          } @else {
            <p class="muted" i18n="@@pluginsPanel.packages.noneGranted">No permissions granted.</p>
          }
          @if (p.requested.length) {
            <button type="button" class="btn" (click)="permissions()" data-testid="plugin-permissions-edit" i18n="@@pluginsPanel.detail.changePermissions">Change permissions…</button>
          }
        } @else {
          <p class="muted" i18n="@@pluginsPanel.detail.noPackage">This plugin does not come from a package, so it has no permissions to review.</p>
        }
      </section>
    </div>
  `,
  styles: `
    :host { position: absolute; z-index: 3; top: 0; right: 0; bottom: 0; width: 340px; min-height: 0; display: flex; box-shadow: var(--shadow); flex-direction: column; border-left: 1px solid var(--border); background: var(--bg-1); }
    .head { display: flex; align-items: flex-start; gap: 8px; padding: 12px 14px; border-bottom: 1px solid var(--border); }
    .titles { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
    .name { margin: 0; font-size: var(--fs-lg); font-weight: 600; overflow-wrap: anywhere; }
    .sub { font-size: var(--fs-sm); }
    .ver { font-variant-numeric: tabular-nums; }
    .body { flex: 1; min-height: 0; overflow: auto; padding: 12px 14px; display: flex; flex-direction: column; gap: 16px; }
    h4 { margin: 0 0 6px; font-size: var(--fs-sm); font-weight: 600; color: var(--muted); }
    p { margin: 0 0 4px; font-size: var(--fs-sm); line-height: 1.4; }
    .warn { color: var(--warn-text); }
    .danger { color: var(--danger-text, var(--lvl-error)); overflow-wrap: anywhere; }
    .id { font-size: var(--fs-xs); overflow-wrap: anywhere; }
    .caps, .perms { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
    .caps li { display: flex; align-items: center; gap: 6px; font-size: var(--fs-xs); overflow-wrap: anywhere; }
    .perms li { font-size: var(--fs-sm); }
    .muted { color: var(--muted); }
  `,
})
export class PluginDetail {
  readonly plugin = input.required<PluginRow>();
  readonly closed = output<void>();
  private readonly packages = inject(PluginPackagesStore);
  private readonly dialogs = inject(DialogService);
  protected readonly groups = computed(() => provideGroups(this.plugin().contributes));
  protected readonly status = computed(() => statusLabel(this.plugin().status));
  /** The package this plugin was installed from, if it was. */
  protected readonly pkg = computed(() => this.packages.plugins().find((p) => p.id === this.plugin().id) ?? null);
  protected readonly title = permissionTitle;
  protected readonly signature = signatureText;
  protected readonly state = stateLabel;

  protected permissions(): void {
    const p = this.pkg();
    if (!p) return;
    this.dialogs.open(PluginPermissionsDialog, {
      title: $localize`:@@pluginsPanel.packages.permissions.title:Permissions of ${p.name}:name:`,
      inputs: { pluginId: p.id },
    });
  }
}
