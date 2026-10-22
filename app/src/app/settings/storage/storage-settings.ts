import { formatCount } from '../../shared/format/units';
import { ChangeDetectionStrategy, Component, DestroyRef, inject } from '@angular/core';
import { formatBytes } from '../../ai/ai-format';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import { DialogService } from '../../ui/dialog.service';
import { StorageStore } from './storage.store';
import { StorageStores } from './stores-block';
import { StorageCategory, StorageCategoryClearResult, StorageCategoryId } from './storage.types';

/** The category's name in the UI language (the engine's English label for ids this UI does not know). */
export function categoryLabel(c: { id: StorageCategoryId; label: string }): string {
  switch (c.id) {
    case 'bundle-extracts':
      return $localize`:Storage category@@settings.storage.category.bundleExtracts:Opened bundles (temporary copies)`;
    case 'webview-cache':
      return $localize`:Storage category@@settings.storage.category.webviewCache:Web view cache`;
    case 'ai-runtimes-outdated':
      return $localize`:Storage category@@settings.storage.category.aiRuntimesOutdated:Outdated AI runtimes`;
    case 'ai-runtimes':
      return $localize`:Storage category@@settings.storage.category.aiRuntimes:AI runtimes`;
    case 'ai-models':
      return $localize`:Storage category@@settings.storage.category.aiModels:AI models`;
    case 'logs':
      return $localize`:Storage category@@settings.storage.category.logs:IMT logs`;
    case 'error-snapshots':
      return $localize`:Storage category@@settings.storage.category.errorSnapshots:Error snapshots`;
    case 'sessions':
      return $localize`:Storage category@@settings.storage.category.sessions:Session backups`;
    case 'stores':
      return $localize`:Storage category@@settings.storage.category.stores:Persistent stores`;
    default:
      return c.label;
  }
}

/** "≥ 12 MB" when the engine's scan stopped at its limits. */
export function sizeLabel(bytes: number, capped: boolean): string {
  return capped ? `≥ ${formatBytes(bytes)}` : formatBytes(bytes);
}

/** One line per cleared category: what was freed, what stayed and why. */
export function resultLine(r: StorageCategoryClearResult, label: string): string {
  if (r.error) return $localize`:@@settings.storage.result.error:${label}:category:: ${r.error}:error:`;
  const freed = formatBytes(r.freedBytes);
  if (!r.skippedCount) return $localize`:@@settings.storage.result.freed:${label}:category:: freed ${freed}:size:.`;
  const why = r.skipped[0]?.reason ?? '';
  return $localize`:@@settings.storage.result.freedKept:${label}:category:: freed ${freed}:size:, kept ${r.skippedCount}:count: (${why}:reason:).`;
}

/**
 * Settings ▸ Storage: what IMT keeps on this computer, per category, with "Clear cache" for everything
 * IMT can rebuild and a separate, confirmed Clear for each data category. Sizes and what may be removed
 * come from the engine.
 */
@Component({
  selector: 'imt-storage-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [StorageStores],
  template: `
    <div class="section-head">
      <h3 i18n="Settings section@@settings.storage.title">Disk usage</h3>
      <span class="spacer"></span>
      <button class="btn" type="button" [disabled]="store.busy()" (click)="refresh()" i18n="@@settings.storage.refresh">Refresh</button>
      <button class="btn primary" type="button" [disabled]="store.busy() || !store.cacheBytes()" (click)="confirmClearCache()" i18n="@@settings.storage.clearCache">Clear cache…</button>
    </div>
    <p class="desc" i18n="@@settings.storage.desc">What IMT keeps on this computer. The cache is rebuilt when needed; data is yours and is removed one category at a time.</p>

    @if (store.loadError(); as e) {
      <div class="banner err" role="alert">
        <span i18n="@@settings.storage.loadError">Could not measure IMT's storage: {{ e }}</span>
        <button class="btn" type="button" (click)="store.load()" i18n="@@settings.storage.retry">Retry</button>
      </div>
    }
    @if (store.clearing()) {
      <p class="muted" role="status" i18n="@@settings.storage.clearing">Clearing… You can keep working; it continues in the background.</p>
    } @else if (store.loading() && !store.usage()) {
      <p class="muted" role="status" i18n="@@settings.storage.loading">Measuring…</p>
    }
    @if (store.actionError(); as e) {
      <div class="banner err" role="alert">
        <span>{{ e }}</span>
        <button class="btn" type="button" (click)="store.retry()" i18n="@@settings.storage.retry">Retry</button>
        <button class="btn ghost" type="button" (click)="store.dismiss()" i18n="@@settings.storage.dismiss">Dismiss</button>
      </div>
    }
    @if (store.result(); as r) {
      <div class="banner ok" role="status" data-testid="storage-result">
        <div class="lines">
          <strong i18n="@@settings.storage.result.total">Freed {{ bytes(r.freedBytes) }}.</strong>
          @if (r.cancelled) {
            <span i18n="@@settings.storage.result.cancelled">Stopped before everything was cleared.</span>
          }
          @for (c of r.categories; track c.id) {
            <span [class.error]="!!c.error">{{ line(c) }}</span>
          }
        </div>
        <button class="btn ghost" type="button" (click)="store.dismiss()" i18n="@@settings.storage.dismiss">Dismiss</button>
      </div>
    }
    @if (store.revealed(); as p) {
      <p class="muted" role="status" i18n="@@settings.storage.revealed">Opened {{ p }}</p>
    }

    @if (store.usage(); as u) {
      <table aria-label="IMT's storage" i18n-aria-label="@@settings.storage.table.label">
        <thead>
          <tr>
            <th scope="col" i18n="@@settings.storage.col.category">Category</th>
            <th scope="col" class="num" i18n="@@settings.storage.col.size">Size</th>
            <th scope="col" i18n="@@settings.storage.col.location">Location</th>
            <th scope="col"><span class="sr-only" i18n="@@settings.storage.col.actions">Actions</span></th>
          </tr>
        </thead>
        <tbody>
          @for (c of store.categories(); track c.id) {
            <tr [attr.data-category]="c.id">
              <th scope="row">
                <span class="name">{{ label(c) }}</span>
                <span class="badge" [class.cache]="c.kind === 'cache'">{{ c.kind === 'cache' ? cacheLabel : dataLabel }}</span>
                @if (c.note) {
                  <span class="note">{{ c.note }}</span>
                }
              </th>
              <td class="num">
                <span class="size">{{ size(c.bytes, c.capped) }}</span>
                <span class="files" i18n="@@settings.storage.files">{c.files, plural, =1 {1 file} other {{{ fmt(c.files) // i18n(ph: "count") }} files}}</span>
              </td>
              <td class="where">
                @if (c.path) {
                  <span class="path mono" [title]="c.path">{{ c.path }}</span>
                  <button class="btn ghost small" type="button" [disabled]="!c.exists" [attr.aria-label]="showLabel(c)" (click)="store.reveal(c.id)" i18n="@@settings.storage.show">Show in folder</button>
                } @else {
                  <span class="muted" i18n="@@settings.storage.noFolder">Not available on this computer</span>
                }
              </td>
              <td class="act">
                @if (c.kind === 'data' && c.clearable) {
                  <button class="btn small" type="button" [disabled]="store.busy() || !c.clearableBytes" [attr.aria-label]="clearLabel(c)" (click)="confirmClear(c)" i18n="@@settings.storage.clear">Clear…</button>
                }
              </td>
            </tr>
          }
        </tbody>
        <tfoot>
          <tr>
            <th scope="row" i18n="@@settings.storage.total">Total</th>
            <td class="num">{{ size(u.totalBytes, u.capped) }}</td>
            <td colspan="2">
              @if (u.capped) {
                <span class="muted" i18n="@@settings.storage.capped">Some folders are too large to measure fully; sizes marked ≥ are at least this much.</span>
              }
            </td>
          </tr>
        </tfoot>
      </table>
    }
    <imt-storage-stores />
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; }
    .section-head { display: flex; align-items: center; gap: 8px; }
    h3 { margin: 0; font-size: var(--fs-lg); font-weight: 600; }
    .spacer { flex: 1; }
    .desc { margin: 0 0 4px; color: var(--muted); line-height: 1.45; }
    .muted { margin: 0; color: var(--muted); }
    .banner { display: flex; align-items: flex-start; gap: 10px; padding: 8px 10px; border-radius: 6px; border: 1px solid var(--border-3); background: var(--bg-3); color: var(--text-2); }
    .banner > span, .banner .lines { flex: 1; }
    .banner.err { border-color: var(--danger-text); color: var(--danger-text); }
    .banner.ok { border-color: var(--ok); }
    .lines { display: flex; flex-direction: column; gap: 2px; }
    .lines .error { color: var(--danger-text); }
    table { width: 100%; border-collapse: collapse; }
    th, td { padding: 6px 8px; text-align: start; vertical-align: top; border-bottom: 1px solid var(--border); }
    thead th { font-size: var(--fs-xs); font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; color: var(--muted); }
    tbody th { font-weight: 500; color: var(--text); }
    .name { margin-inline-end: 6px; }
    .badge { display: inline-block; padding: 0 6px; border-radius: 8px; font-size: var(--fs-xs); color: var(--text-2); border: 1px solid var(--border-3); }
    .badge.cache { border-color: var(--accent); color: var(--text); }
    .note { display: block; margin-top: 2px; font-size: var(--fs-sm); font-weight: 400; color: var(--muted); }
    .num { text-align: end; white-space: nowrap; }
    .files { display: block; font-size: var(--fs-sm); color: var(--muted); }
    .where { max-width: 320px; }
    .path { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--fs-sm); color: var(--text-2); }
    .act { text-align: end; white-space: nowrap; }
    .btn.small { padding: 2px 8px; font-size: var(--fs-sm); }
    tfoot th, tfoot td { border-bottom: 0; font-weight: 600; }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  `,
})
export class StorageSettings {
  protected readonly fmt = formatCount;
  protected readonly store = inject(StorageStore);
  private readonly dialogs = inject(DialogService);
  protected readonly cacheLabel = $localize`:Kind of storage IMT can rebuild@@settings.storage.kind.cache:cache`;
  protected readonly dataLabel = $localize`:Kind of storage that belongs to the user@@settings.storage.kind.data:data`;

  constructor() {
    void this.store.load();
    // Settings may close while a clear runs; it goes on in Jobs and tells when it is done.
    inject(DestroyRef).onDestroy(() => this.store.detach());
  }

  protected refresh(): void {
    void this.store.load();
    void this.store.loadStores();
  }

  protected label(c: StorageCategory): string {
    return categoryLabel(c);
  }

  protected size(bytes: number, capped: boolean): string {
    return sizeLabel(bytes, capped);
  }

  protected bytes(n: number): string {
    return formatBytes(n);
  }

  protected line(r: StorageCategoryClearResult): string {
    const c = this.store.categories().find((x) => x.id === r.id);
    return resultLine(r, categoryLabel(c ?? { id: r.id, label: r.id }));
  }

  protected showLabel(c: StorageCategory): string {
    return $localize`:Accessible name; starts with the visible button text@@settings.storage.show.label:Show in folder (${categoryLabel(c)}:category:)`;
  }

  protected clearLabel(c: StorageCategory): string {
    return $localize`:Accessible name; starts with the visible button text@@settings.storage.clear.label:Clear… (${categoryLabel(c)}:category:)`;
  }

  /** "Clear cache": one confirmation listing every cache category and how much goes. */
  protected async confirmClearCache(): Promise<void> {
    const cache = this.store.cache();
    if (!cache.length) return;
    const total = formatBytes(this.store.cacheBytes());
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:Dialog title@@settings.storage.clearCache.title:Clear cache?`,
      width: 480,
      inputs: {
        message: $localize`:@@settings.storage.clearCache.message:IMT removes ${total}:size: of files it can rebuild. Your logs, models and sessions are not touched.`,
        detail: cache.map((c) => `${categoryLabel(c)} — ${sizeLabel(c.clearableBytes, c.capped)}`).join(' · '),
        confirmLabel: $localize`:@@settings.storage.clearCache.confirm:Clear cache`,
        danger: true,
      },
    });
    if ((await ref.closed) === true) await this.store.clearCache();
  }

  /** A data category: its own confirmation, naming what is deleted for good and where. */
  protected async confirmClear(c: StorageCategory): Promise<void> {
    const name = categoryLabel(c);
    const size = sizeLabel(c.clearableBytes, c.capped);
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:Dialog title@@settings.storage.clearData.title:Remove ${name}:category:?`,
      width: 480,
      inputs: {
        message: $localize`:@@settings.storage.clearData.message:This permanently deletes ${size}:size: of ${name}:category:. It cannot be undone.` + (c.note ? ` ${c.note}` : ''),
        detail: c.path ?? '',
        confirmLabel: $localize`:@@settings.storage.clearData.confirm:Remove`,
        danger: true,
      },
    });
    if ((await ref.closed) === true) await this.store.clear([c.id]);
  }
}
