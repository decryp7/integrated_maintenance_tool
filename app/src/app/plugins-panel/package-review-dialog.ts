import { ChangeDetectionStrategy, Component, computed, effect, inject, Injectable, untracked } from '@angular/core';
import { PermissionDto } from '../core/api.types';
import { ToolError } from '../tools/elevation-hint';
import { DIALOG_REF, DialogRef, DialogService } from '../ui/dialog.service';
import { permissionTitle, refusalLiftedByTrust, signatureText } from './package-labels';
import { PluginPackagesStore } from './plugin-packages.store';

/**
 * The review before installing a package: who made it, whether its signature is trusted, what it replaces and the
 * permissions it asks for (each can be withheld). Install sends the reviewed digest, so a package that changed since
 * is refused by the shell. Closes itself when the review ends (installed or cancelled).
 */
@Component({
  selector: 'debrief-package-review-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ToolError],
  template: `
    @if (store.review(); as r) {
      <div class="top">
        <strong data-testid="package-name">{{ r.pkg.name }}</strong>
        <span class="mono faint">v{{ r.pkg.version }}</span>
        <span class="mono faint id">{{ r.pkg.id }}</span>
      </div>
      <dl class="facts">
        <dt i18n="@@pluginsPanel.packages.review.publisher">Publisher</dt>
        <dd data-testid="package-publisher">{{ r.pkg.publisher }}</dd>
        <dt i18n="@@pluginsPanel.packages.review.signature">Signature</dt>
        <dd><span class="sig" [class]="'sig ' + r.pkg.signature.kind" data-testid="package-signature">{{ signature(r.pkg.signature) }}</span></dd>
      </dl>
      @if (r.pkg.description) { <p class="desc">{{ r.pkg.description }}</p> }
      @if (r.pkg.warning) { <div class="warn" role="note" data-testid="package-warning">{{ r.pkg.warning }}</div> }
      @if (r.pkg.refusal; as refusal) {
        <div class="refusal" role="alert" data-testid="package-refusal">
          <span>{{ refusal.message }}</span>
          @if (liftedByTrust(refusal.reason)) {
            <span i18n="@@pluginsPanel.packages.review.trustHint">To install it, add its publisher's key under Settings → Plugin trust.</span>
          }
        </div>
      }
      @if (r.pkg.update; as u) {
        <div class="update" data-testid="package-update">
          <span i18n="@@pluginsPanel.packages.review.replaces">Replaces version {{ u.fromVersion }}.</span>
          @if (u.added.length) { <span class="added" i18n="@@pluginsPanel.packages.review.added">Newly requested: {{ titles(u.added) }}</span> }
          @if (u.removed.length) { <span i18n="@@pluginsPanel.packages.review.removed">No longer requested: {{ titles(u.removed) }}</span> }
        </div>
      }
      <fieldset class="perms" [disabled]="!!r.pkg.refusal">
        <legend i18n="@@pluginsPanel.packages.review.permissions">Permissions</legend>
        @for (p of r.pkg.permissions; track p.permission) {
          <label class="perm">
            <input type="checkbox" [checked]="r.approved.includes(p.permission)" (change)="store.toggleApproved(p.permission)" [attr.data-permission]="p.permission" />
            <span class="what">
              <strong>{{ title(p.permission) }}</strong>
              @if (isAdded(p.permission)) { <span class="badge new" i18n="@@pluginsPanel.packages.review.new">new</span> }
              <span class="allows">{{ p.allows }}</span>
              @if (p.reason) { <span class="reason" i18n="@@pluginsPanel.packages.review.reason">Publisher's reason: {{ p.reason }}</span> }
            </span>
          </label>
        } @empty {
          <p class="faint" i18n="@@pluginsPanel.packages.review.noPermissions">This plugin asks for no permissions.</p>
        }
        @if (r.pkg.permissions.length) {
          <p class="faint note" i18n="@@pluginsPanel.packages.review.permissionsNote">Unchecked permissions are not granted; you can change them later in the Plugins panel.</p>
        }
      </fieldset>
      <debrief-tool-error [message]="store.error()" (dismissed)="store.dismissError()" />
      <div class="foot">
        <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@pluginsPanel.packages.review.cancel">Cancel</button>
        <button type="button" class="btn primary" [disabled]="!store.canInstall()" (click)="store.install()">{{ store.busy() ?? installLabel() }}</button>
      </div>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    .top { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
    .id { margin-left: auto; font-size: var(--fs-xs); }
    .facts { display: grid; grid-template-columns: max-content 1fr; gap: 4px 12px; margin: 0; font-size: var(--fs-sm); }
    .facts dt { color: var(--muted); }
    .facts dd { margin: 0; }
    .sig { font-weight: 600; }
    .sig.trusted { color: var(--ok); }
    .sig.unknownKey { color: var(--warn-text); }
    .sig.unsigned { color: var(--danger-text); }
    .desc { margin: 0; color: var(--text-2); }
    .warn, .refusal, .update { display: flex; flex-direction: column; gap: 4px; padding: 8px 10px; border-radius: 6px; background: var(--bg-3); font-size: var(--fs-sm); }
    .warn { border: 1px solid var(--lvl-warn); color: var(--warn-text); }
    .refusal { border: 1px solid var(--lvl-error); color: var(--danger-text); }
    .update { border: 1px solid var(--border-2); color: var(--text-2); }
    .added { color: var(--warn-text); }
    .perms { margin: 0; padding: 8px 10px; border: 1px solid var(--border-2); border-radius: 6px; display: flex; flex-direction: column; gap: 8px; }
    .perms legend { padding: 0 4px; color: var(--muted); font-size: var(--fs-sm); }
    .perm { display: flex; gap: 8px; align-items: flex-start; cursor: pointer; }
    .perm input { margin-top: 3px; }
    .what { display: flex; flex-direction: column; gap: 2px; font-size: var(--fs-sm); }
    .allows { color: var(--text-2); }
    .reason { color: var(--muted); font-style: italic; }
    .badge.new { align-self: flex-start; color: var(--warn-text); }
    .note { margin: 0; font-size: var(--fs-xs); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class PackageReviewDialog {
  static readonly dialogId = 'package-review';
  protected readonly ref = inject(DIALOG_REF);
  protected readonly store = inject(PluginPackagesStore);
  protected readonly signature = signatureText;
  protected readonly title = permissionTitle;
  protected readonly liftedByTrust = refusalLiftedByTrust;
  protected readonly installLabel = computed(() =>
    this.store.review()?.pkg.update ? $localize`:@@pluginsPanel.packages.review.update:Update` : $localize`:@@pluginsPanel.packages.review.install:Install`,
  );

  constructor() {
    // Installed or cancelled elsewhere: nothing left to review.
    effect(() => {
      if (!this.store.review()) untracked(() => this.ref.close());
    });
  }

  protected titles(permissions: PermissionDto[]): string {
    return permissions.map(permissionTitle).join(', ');
  }

  protected isAdded(permission: PermissionDto): boolean {
    return !!this.store.review()?.pkg.update?.added.includes(permission);
  }
}

/** Opens the review dialog whenever a review starts (once per review, whichever view or drop started it). Inject it to start it. */
@Injectable({ providedIn: 'root' })
export class PackageReviewOpener {
  private readonly dialogs = inject(DialogService);
  private readonly store = inject(PluginPackagesStore);
  private ref: DialogRef | null = null;

  constructor() {
    effect(() => {
      if (this.store.review()) untracked(() => this.open());
    });
  }

  private open(): void {
    if (this.ref) return;
    const ref = this.dialogs.open(PackageReviewDialog, { title: $localize`:@@pluginsPanel.packages.review.title:Install plugin`, width: 600 });
    this.ref = ref;
    void ref.closed.then(() => {
      this.ref = null;
      if (this.store.review()) this.store.cancelReview();
    });
  }
}
