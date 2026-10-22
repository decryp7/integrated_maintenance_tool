import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';

/** The workspace's error and notice ("Saved Case 7"), as toasts in the toast stack (`ui/toast-stack.ts`). */
@Component({
  selector: 'imt-workspace-toasts',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  host: { style: 'display: contents' },
  template: `
    @if (store.error(); as error) {
      <div class="toast" data-imt-overlay data-toast role="alert">
        <imt-icon name="alert" [size]="15" />
        <span class="ellipsis">{{ error }}</span>
        <button class="icon-btn" aria-label="Dismiss" title="Dismiss" i18n-aria-label="@@app.toast.dismissError" i18n-title="@@app.toast.dismissErrorTitle" (click)="store.dismissError()"><imt-icon name="close" [size]="14" /></button>
      </div>
    }
    @if (store.notice(); as notice) {
      <div class="toast info" data-imt-overlay data-toast role="status" (mouseenter)="store.holdNotice()" (focusin)="store.holdNotice()" (mouseleave)="store.releaseNotice()" (focusout)="store.releaseNotice()">
        <imt-icon name="check" [size]="15" />
        <span class="ellipsis">{{ notice }}</span>
        @if (store.noticeAction(); as action) {
          <button type="button" class="btn ghost act" data-testid="notice-action" (click)="store.runNoticeAction()">{{ action.label }}</button>
        }
        <button class="icon-btn" aria-label="Dismiss" title="Dismiss" i18n-aria-label="@@app.toast.dismissNotice" i18n-title="@@app.toast.dismissNoticeTitle" (click)="store.setNotice(null)"><imt-icon name="close" [size]="14" /></button>
      </div>
    }
  `,
  styles: `
    .toast {
      max-width: 520px; box-sizing: border-box;
      display: flex; align-items: center; gap: 10px; padding: 8px 8px 8px 12px; border-radius: 8px;
      background: var(--tooltip-bg); border: 1px solid var(--lvl-error); color: var(--text); box-shadow: var(--shadow);
    }
    .toast imt-icon { color: var(--lvl-error); }
    .toast.info { border-color: var(--ok); }
    .toast.info imt-icon { color: var(--ok); }
    .act { flex: none; height: 24px; padding: 0 8px; }
  `,
})
export class WorkspaceToasts {
  protected readonly store = inject(WorkspaceStore);
}
