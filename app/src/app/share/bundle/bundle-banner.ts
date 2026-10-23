import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { formatDateTime } from '../../core/format';
import { WorkspaceStore } from '../../core/workspace.store';
import { Icon } from '../../shared/icon';
import { BundleService } from './bundle.service';

/** Shown under the title bar while a bundle is open: it is read-only until saved as a project. */
@Component({
  selector: 'debrief-bundle-banner',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  host: { style: 'display: contents' },
  template: `
    @if (bundles.opened(); as b) {
      <div class="banner info" role="status" data-testid="bundle-banner">
        <debrief-icon name="folder" [size]="14" />
        <span class="msg" i18n="@@share.bundleBanner.opened">Opened from <strong>{{ b.name }}</strong> (read-only){{ details() }}</span>
        <button type="button" class="btn" (click)="bundles.saveAsProject()" i18n="@@share.bundleBanner.saveAsProject">Save as project…</button>
        <button type="button" class="btn ghost" (click)="bundles.close()" i18n="@@share.bundleBanner.close">Close</button>
      </div>
    }
  `,
  styles: `
    .banner { flex: none; display: flex; align-items: center; gap: 10px; padding: 6px 12px; font-size: var(--fs-sm); border-bottom: 1px solid var(--border); background: var(--bg-3); color: var(--text); border-left: 3px solid var(--accent); }
    .msg { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  `,
})
export class BundleBanner {
  protected readonly bundles = inject(BundleService);
  private readonly store = inject(WorkspaceStore);

  protected readonly details = computed(() => {
    const m = this.bundles.opened()?.manifest;
    if (!m) return '';
    const parts = [m.redacted ? $localize`:@@share.bundleBanner.redacted:redacted` : '', m.createdAt ? $localize`:@@share.bundleBanner.created:created ${formatDateTime(m.createdAt, this.store.timezone())}:when:` : '', m.debriefVersion ? `Debrief ${m.debriefVersion}` : ''].filter((x) => !!x);
    return parts.length ? ` · ${parts.join(' · ')}` : '';
  });
}
