import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { Icon } from '../../shared/icon';
import { RedactionService } from './redaction.service';

/** Shown under the title bar while the window's redacted view is on. */
@Component({
  selector: 'imt-redaction-banner',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  host: { style: 'display: contents' },
  template: `
    @if (redaction.on()) {
      <div class="banner info" role="status" data-testid="redacted-banner">
        <imt-icon name="check" [size]="14" />
        <span class="msg" i18n="@@share.redactionBanner.message">Redacted view: secrets and personal data are masked in the grid, facets and context.</span>
        <button type="button" class="btn" (click)="redaction.openSettings()" i18n="@@share.redactionBanner.settings">Settings…</button>
        <button type="button" class="btn ghost" [disabled]="redaction.busy()" (click)="redaction.set(false)" i18n="@@share.redactionBanner.turnOff">Turn off</button>
      </div>
    }
  `,
  styles: `
    .banner { flex: none; display: flex; align-items: center; gap: 10px; padding: 6px 12px; font-size: 12px; border-bottom: 1px solid var(--border); background: var(--bg-3); color: var(--text); border-left: 3px solid var(--accent); }
    .msg { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  `,
})
export class RedactionBanner {
  protected readonly redaction = inject(RedactionService);
}
