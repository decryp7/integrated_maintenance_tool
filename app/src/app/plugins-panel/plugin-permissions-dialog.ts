import { ChangeDetectionStrategy, Component, computed, inject, input, linkedSignal } from '@angular/core';
import { PermissionDto } from '../core/api.types';
import { ToolError } from '../tools/elevation-hint';
import { DIALOG_REF } from '../ui/dialog.service';
import { permissionTitle } from './package-labels';
import { PluginPackagesStore } from './plugin-packages.store';

/** Grants or revokes the permissions an installed plugin asks for; a running plugin restarts with the new set at once. */
@Component({
  selector: 'imt-plugin-permissions-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ToolError],
  template: `
    @if (plugin(); as p) {
      <p class="intro" i18n="@@pluginsPanel.packages.permissions.intro">Choose what {{ p.name }} may do. The plugin restarts at once with the new permissions.</p>
      <fieldset class="perms">
        <legend i18n="@@pluginsPanel.packages.permissions.legend">Permissions</legend>
        @for (r of p.requested; track r.permission) {
          <label class="perm">
            <input type="checkbox" [checked]="granted().includes(r.permission)" (change)="toggle(r.permission)" [attr.data-permission]="r.permission" />
            <span class="what">
              <strong>{{ title(r.permission) }}</strong>
              <span class="allows">{{ r.allows }}</span>
              @if (r.reason) { <span class="reason" i18n="@@pluginsPanel.packages.review.reason">Publisher's reason: {{ r.reason }}</span> }
            </span>
          </label>
        } @empty {
          <p class="faint" i18n="@@pluginsPanel.packages.review.noPermissions">This plugin asks for no permissions.</p>
        }
      </fieldset>
      <imt-tool-error [message]="store.error()" (dismissed)="store.dismissError()" />
      <div class="foot">
        <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@pluginsPanel.packages.permissions.cancel">Cancel</button>
        <button type="button" class="btn primary" [disabled]="!!store.busy() || !changed()" (click)="save(p.id)" i18n="@@pluginsPanel.packages.permissions.save">Apply and restart plugin</button>
      </div>
    } @else {
      <p class="faint" i18n="@@pluginsPanel.packages.permissions.gone">This plugin is no longer installed.</p>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    .intro { margin: 0; color: var(--text-2); }
    .perms { margin: 0; padding: 8px 10px; border: 1px solid var(--border-2); border-radius: 6px; display: flex; flex-direction: column; gap: 8px; }
    .perms legend { padding: 0 4px; color: var(--muted); font-size: var(--fs-sm); }
    .perm { display: flex; gap: 8px; align-items: flex-start; cursor: pointer; }
    .perm input { margin-top: 3px; }
    .what { display: flex; flex-direction: column; gap: 2px; font-size: var(--fs-sm); }
    .allows { color: var(--text-2); }
    .reason { color: var(--muted); font-style: italic; }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class PluginPermissionsDialog {
  protected readonly ref = inject(DIALOG_REF);
  protected readonly store = inject(PluginPackagesStore);
  protected readonly title = permissionTitle;
  readonly pluginId = input.required<string>();
  protected readonly plugin = computed(() => this.store.plugins().find((p) => p.id === this.pluginId()) ?? null);
  /** The edited set, starting from what is granted now. */
  protected readonly granted = linkedSignal<PermissionDto[]>(() => this.plugin()?.granted ?? []);
  protected readonly changed = computed(() => {
    const now = new Set(this.plugin()?.granted ?? []);
    const next = this.granted();
    return next.length !== now.size || next.some((p) => !now.has(p));
  });

  protected toggle(permission: PermissionDto): void {
    this.granted.update((g) => (g.includes(permission) ? g.filter((p) => p !== permission) : [...g, permission]));
  }

  protected async save(id: string): Promise<void> {
    const order = (this.plugin()?.requested ?? []).map((r) => r.permission);
    if (await this.store.setPermissions(id, order.filter((p) => this.granted().includes(p)))) this.ref.close();
  }
}
