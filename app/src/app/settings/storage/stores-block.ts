import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { UiPrefsStore } from '../../core/workspace/ui-prefs.store';
import { formatFull } from '../../core/format';
import { formatBytes, formatCount, formatRelativeTime } from '../../shared/format/units';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import { DialogService } from '../../ui/dialog.service';
import { StorageStore } from './storage.store';
import { StoreFolder } from './storage.types';
import { notSelectableReason, parseAutoCleanDays, shortStoreName, stateLabel } from './stores-format';

/**
 * Settings ▸ Storage ▸ Project stores: the store folders on disk with their state, clean-up of the orphaned ones (after
 * a confirmation; the engine checks each again) and the automatic clean-up setting. What may be removed is decided in
 * Rust; this only presents it.
 */
@Component({
  selector: 'imt-storage-stores',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="head">
      <h4 i18n="@@settings.storage.stores.title">Project stores</h4>
      <span class="spacer"></span>
      <button class="btn small" type="button" [disabled]="store.storesClearing() || !store.selected().length" data-testid="stores-clean-selected" (click)="confirm(store.selected())" i18n="@@settings.storage.stores.cleanSelected">Clean up selected…</button>
      <button class="btn small" type="button" [disabled]="store.storesClearing() || !store.orphanedFolders().length" data-testid="stores-clean-all" (click)="confirm(allOrphaned())" i18n="@@settings.storage.stores.cleanAll">Clean up all orphaned</button>
    </div>
    <p class="desc" i18n="@@settings.storage.stores.desc">The event stores IMT keeps on disk. Only orphaned stores, which nothing references any more, can be removed; a store that is open is never touched.</p>

    @if (store.storesError(); as e) {
      <div class="banner err" role="alert">
        <span i18n="@@settings.storage.stores.loadError">Could not list the project stores: {{ e }}</span>
        <button class="btn" type="button" (click)="store.loadStores()" i18n="@@settings.storage.retry">Retry</button>
      </div>
    }
    @if (store.storesActionError(); as e) {
      <div class="banner err" role="alert" data-testid="stores-error">
        <span>{{ e }}</span>
        <button class="btn" type="button" (click)="store.dismissStores()" i18n="@@settings.storage.dismiss">Dismiss</button>
      </div>
    }
    @if (store.storesResult(); as r) {
      <div class="banner ok" role="status" data-testid="stores-result">
        <div class="lines">
          <strong i18n="@@settings.storage.stores.freed">Freed {{ bytes(r.freedBytes) }}.</strong>
          @if (r.cancelled) {
            <span i18n="@@settings.storage.stores.cancelled">Stopped before every store was handled.</span>
          }
          @for (s of r.skipped; track s.id) {
            <span class="skipped" i18n="@@settings.storage.stores.skipped">Kept {{ short(s.id) }}: {{ s.reason }}.</span>
          }
        </div>
        <button class="btn ghost" type="button" (click)="store.dismissStores()" i18n="@@settings.storage.dismiss">Dismiss</button>
      </div>
    }
    @if (store.storesLoading() && !store.stores()) {
      <p class="muted" role="status" i18n="@@settings.storage.stores.loading">Measuring…</p>
    }

    @if (store.stores(); as s) {
      @if (!s.folders.length) {
        <p class="muted" data-testid="stores-empty" i18n="@@settings.storage.stores.empty">No stores on disk</p>
      } @else {
        <table aria-label="Project stores" i18n-aria-label="@@settings.storage.stores.table.label">
          <thead>
            <tr>
              <th scope="col"><span class="sr-only" i18n="@@settings.storage.stores.col.select">Select</span></th>
              <th scope="col" i18n="@@settings.storage.stores.col.store">Store</th>
              <th scope="col" class="num" i18n="@@settings.storage.stores.col.size">Size</th>
              <th scope="col" i18n="@@settings.storage.stores.col.lastUsed">Last used</th>
              <th scope="col" i18n="@@settings.storage.stores.col.state">State</th>
            </tr>
          </thead>
          <tbody>
            @for (f of s.folders; track f.id) {
              <tr [attr.data-store]="f.id">
                <td>
                  <input
                    type="checkbox"
                    [checked]="store.selected().includes(f.id)"
                    [disabled]="f.state !== 'orphaned' || store.storesClearing()"
                    [attr.title]="why(f)"
                    [attr.aria-label]="selectLabel(f)"
                    (change)="store.toggleStore(f.id, $any($event.target).checked)"
                  />
                </td>
                <th scope="row" class="mono" [title]="f.id">{{ short(f.id) }}</th>
                <td class="num">
                  <span class="size">{{ bytes(f.bytes) }}</span>
                  <span class="files" i18n="@@settings.storage.stores.files">{f.files, plural, =1 {1 file} other {{{ fmt(f.files) // i18n(ph: "count") }} files}}</span>
                </td>
                <td [title]="exact(f)">{{ lastUsed(f) }}</td>
                <td><span class="badge" [attr.data-state]="f.state">{{ state(f) }}</span></td>
              </tr>
            }
          </tbody>
          <tfoot>
            <tr>
              <th scope="row" colspan="2" i18n="@@settings.storage.stores.total">Total</th>
              <td class="num">{{ bytes(s.totalBytes) }}</td>
              <td colspan="2" data-testid="stores-orphaned-total" i18n="@@settings.storage.stores.orphanedTotal">orphaned: {{ bytes(s.orphanedBytes) }} can be freed</td>
            </tr>
          </tfoot>
        </table>
      }
    }

    <div class="policy">
      <label for="stores-auto-days" i18n="@@settings.storage.stores.policy.label">Automatically remove orphaned stores unused for N days (0 = off)</label>
      <input
        id="stores-auto-days"
        class="input days"
        type="number"
        min="0"
        max="365"
        step="1"
        data-testid="stores-auto-days"
        [value]="store.policy()?.autoCleanDays ?? 0"
        [disabled]="!store.policy()"
        [attr.aria-invalid]="!!store.policyError()"
        aria-describedby="stores-auto-days-error"
        (change)="setDays($any($event.target).value)"
      />
      @if (store.policyError(); as e) {
        <p class="err" id="stores-auto-days-error" role="alert" data-testid="stores-policy-error">{{ e }}</p>
      }
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; margin-top: 16px; }
    .head { display: flex; align-items: center; gap: 8px; }
    h4 { margin: 0; font-size: var(--fs-md); font-weight: 600; }
    .spacer { flex: 1; }
    .desc { margin: 0; color: var(--muted); line-height: 1.45; }
    .muted { margin: 0; color: var(--muted); }
    .err { margin: 0; color: var(--danger-text); }
    .banner { display: flex; align-items: flex-start; gap: 10px; padding: 8px 10px; border-radius: 6px; border: 1px solid var(--border-3); background: var(--bg-3); color: var(--text-2); }
    .banner > span, .banner .lines { flex: 1; }
    .banner.err { border-color: var(--danger-text); color: var(--danger-text); }
    .banner.ok { border-color: var(--ok); }
    .lines { display: flex; flex-direction: column; gap: 2px; }
    table { width: 100%; border-collapse: collapse; }
    th, td { padding: 6px 8px; text-align: start; vertical-align: top; border-bottom: 1px solid var(--border); }
    thead th { font-size: var(--fs-sm); font-weight: 600; color: var(--muted); }
    tbody th { font-weight: 500; color: var(--text); }
    .num { text-align: end; white-space: nowrap; }
    .files { display: block; font-size: var(--fs-sm); color: var(--muted); }
    .badge { display: inline-block; padding: 0 6px; border-radius: 8px; font-size: var(--fs-xs); color: var(--text-2); border: 1px solid var(--border-3); }
    .badge[data-state='orphaned'] { border-color: var(--accent); color: var(--text); }
    tfoot th, tfoot td { border-bottom: 0; font-weight: 600; }
    .policy { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
    .days { width: 80px; }
    .btn.small { padding: 2px 8px; font-size: var(--fs-sm); }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  `,
})
export class StorageStores {
  protected readonly store = inject(StorageStore);
  private readonly dialogs = inject(DialogService);
  private readonly prefs = inject(UiPrefsStore);
  protected readonly fmt = formatCount;
  protected readonly allOrphaned = computed(() => this.store.orphanedFolders().map((f) => f.id));

  constructor() {
    void this.store.loadStores();
    void this.store.loadPolicy();
  }

  protected bytes(n: number): string {
    return formatBytes(n);
  }
  protected short(id: string): string {
    return shortStoreName(id);
  }
  protected state(f: StoreFolder): string {
    return stateLabel(f.state);
  }
  protected why(f: StoreFolder): string | null {
    return notSelectableReason(f.state);
  }
  protected lastUsed(f: StoreFolder): string {
    return formatRelativeTime(f.lastUsedMs, Date.now());
  }
  protected exact(f: StoreFolder): string {
    return f.lastUsedMs === null ? '' : formatFull(f.lastUsedMs, this.prefs.timezone());
  }
  protected selectLabel(f: StoreFolder): string {
    const name = shortStoreName(f.id);
    return $localize`:Accessible name of a checkbox@@settings.storage.stores.select.label:Select ${name}:store:`;
  }

  /** Validates the number before sending it; the engine checks again. */
  protected setDays(raw: string): void {
    const parsed = parseAutoCleanDays(raw);
    if ('error' in parsed) this.store.setPolicyError(parsed.error);
    else void this.store.setAutoCleanDays(parsed.days);
  }

  /** The confirmation lists the count and size and says the data cannot be recovered; then the engine removes them. */
  protected async confirm(ids: readonly string[]): Promise<void> {
    const folders = this.store.orphanedFolders().filter((f) => ids.includes(f.id));
    if (!folders.length) return;
    const count = folders.length;
    const size = formatBytes(folders.reduce((n, f) => n + f.bytes, 0));
    const message =
      count === 1
        ? $localize`:@@settings.storage.stores.confirm.one:This permanently deletes 1 orphaned store (${size}:size:). It cannot be recovered.`
        : $localize`:@@settings.storage.stores.confirm.many:This permanently deletes ${count}:count: orphaned stores (${size}:size:). They cannot be recovered.`;
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:Dialog title@@settings.storage.stores.confirm.title:Clean up project stores?`,
      width: 480,
      inputs: {
        message,
        detail: folders.map((f) => shortStoreName(f.id)).join(' · '),
        confirmLabel: $localize`:@@settings.storage.stores.confirm.ok:Clean up`,
        danger: true,
      },
    });
    if ((await ref.closed) === true) await this.store.clearStores(folders.map((f) => f.id));
  }
}
