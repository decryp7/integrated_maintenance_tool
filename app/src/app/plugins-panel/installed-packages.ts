import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { InstalledPluginDto, PermissionDto } from '../core/api.types';
import { ToolError } from '../tools/elevation-hint';
import { DialogService } from '../ui/dialog.service';
import { permissionTitle, signatureText, stateLabel } from './package-labels';
import { PackageReviewOpener } from './package-review-dialog';
import { PluginPackagesStore } from './plugin-packages.store';
import { PluginPermissionsDialog } from './plugin-permissions-dialog';

/** Installing plugins from `.imtplugin` packages: the install bar, safe-mode and restart notices, damaged records. Installed packages are rows of the plugins table. */
@Component({
  selector: 'imt-installed-packages',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ToolError],
  template: `
    <div class="bar">
      <span class="faint hint" i18n="@@pluginsPanel.packages.dropHint">Or drop a .imtplugin file on the window. You review the publisher and the permissions before anything is installed.</span>
      <span class="grow"></span>
      <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.pickAndReview()" i18n="@@pluginsPanel.packages.installFromFile">Install from file…</button>
    </div>
    @if (store.safeMode()) {
      <div class="note" role="status" data-testid="packages-safe-mode" i18n="@@pluginsPanel.packages.safeMode">Safe mode: third-party plugins are not loaded. Restart normally to use them.</div>
    }
    @if (!store.review()) {
      <imt-tool-error [message]="store.error()" [retry]="store.retry() !== null" (retried)="store.retryFailed()" (dismissed)="store.dismissError()" />
    }
    @if (store.showRestart()) {
      <div class="restart" role="status" data-testid="packages-restart">
        <span i18n="@@pluginsPanel.packages.restartNote">Changes apply after a restart.</span>
        <button type="button" class="btn primary" (click)="store.restart()" i18n="@@pluginsPanel.packages.restartApply">Restart to apply</button>
      </div>
    }
    @if (store.damaged().length) {
      <ul class="list" aria-label="Damaged plugin records" i18n-aria-label="@@pluginsPanel.packages.damaged">
        @for (d of store.damaged(); track d.id) {
          <li class="package damaged" data-testid="package-damaged">
            <div class="top"><strong class="mono">{{ d.id }}</strong></div>
            <div class="problem">{{ d.message }}</div>
            <div class="actions">
              <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.remove(d.id)" [attr.aria-label]="removeLabel(d.id)" i18n="@@pluginsPanel.packages.remove">Remove</button>
            </div>
          </li>
        }
      </ul>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; }
    .bar { display: flex; align-items: center; gap: 12px; }
    .grow { flex: 1; }
    .hint { margin: 0; font-size: var(--fs-sm); }
    .note { padding: 8px 10px; border: 1px solid var(--lvl-warn); border-radius: 6px; background: var(--bg-3); color: var(--warn-text); font-size: var(--fs-sm); }
    .restart { display: flex; align-items: center; gap: 12px; padding: 8px 10px; border: 1px solid var(--accent); border-radius: 6px; background: var(--bg-3); }
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
    .package { padding: 8px 10px; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); display: flex; flex-direction: column; gap: 6px; }
    .package[data-state='quarantined'], .package.damaged { border-color: var(--lvl-error); }
    .package[data-state='unavailable'] { border-color: var(--lvl-warn); }
    .top { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
    .id { margin-left: auto; font-size: var(--fs-xs); }
    .badges { display: flex; gap: 6px; flex-wrap: wrap; }
    .sig.trusted { color: var(--ok); }
    .sig.unknownKey { color: var(--warn-text); }
    .sig.unsigned { color: var(--danger-text); }
    .st.running, .st.ready { color: var(--ok); }
    .st.quarantined { color: var(--danger-text); }
    .st.disabled { color: var(--muted); }
    .st.pendingRestart, .st.unavailable { color: var(--warn-text); }
    .desc, .granted { font-size: var(--fs-sm); color: var(--text-2); }
    .warn { font-size: var(--fs-sm); color: var(--warn-text); }
    .problem { font-size: var(--fs-sm); color: var(--danger-text); overflow-wrap: anywhere; }
    .actions { display: flex; gap: 6px; flex-wrap: wrap; }
  `,
})
export class InstalledPackages {
  protected readonly store = inject(PluginPackagesStore);
  private readonly dialogs = inject(DialogService);
  protected readonly signature = signatureText;
  protected readonly state = stateLabel;

  constructor() {
    // Reviews started here or by a drop open their dialog.
    inject(PackageReviewOpener);
    void this.store.load();
  }

  protected titles(permissions: PermissionDto[]): string {
    return permissions.map(permissionTitle).join(', ');
  }

  protected permissions(p: InstalledPluginDto): void {
    this.dialogs.open(PluginPermissionsDialog, {
      title: $localize`:@@pluginsPanel.packages.permissions.title:Permissions of ${p.name}:name:`,
      inputs: { pluginId: p.id },
    });
  }

  protected enableLabel(p: InstalledPluginDto): string {
    return $localize`:@@pluginsPanel.packages.enableAria:Enable ${p.name}:name:`;
  }
  protected disableLabel(p: InstalledPluginDto): string {
    return $localize`:@@pluginsPanel.packages.disableAria:Disable ${p.name}:name:`;
  }
  protected permissionsLabel(p: InstalledPluginDto): string {
    return $localize`:@@pluginsPanel.packages.permissionsAria:Permissions of ${p.name}:name:`;
  }
  protected removeLabel(name: string): string {
    return $localize`:@@pluginsPanel.packages.removeAria:Remove ${name}:name:`;
  }
}
