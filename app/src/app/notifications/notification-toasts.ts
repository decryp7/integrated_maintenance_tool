import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { Icon } from '../shared/icon';
import { actionLabel, NoticeActions, NOTIFICATIONS_VIEW } from './notice-actions';
import { NotificationsStore } from './notifications.store';
import { Notice, NoticeAction } from './notifications.types';
import { titleWithCount } from './notifications-text';

/**
 * Previews of new notification-centre messages (newest at the bottom) with the message's own action and "Show in
 * Notifications". Mounted once over the app; they go after a few seconds, the message stays in Notifications.
 */
@Component({
  selector: 'imt-notification-toasts',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  host: { 'data-snapshot-ignore': '' },
  template: `
    <div class="stack" role="region" aria-label="New notifications" i18n-aria-label="@@notifications.toasts.region" aria-live="polite">
      @for (t of store.toasts(); track t.id) {
        <div class="toast" data-imt-overlay [attr.data-severity]="t.severity" data-testid="notice-toast" (mouseenter)="store.holdToast(t.id)" (focusin)="store.holdToast(t.id)" (mouseleave)="leave(t.id, $event)" (focusout)="leave(t.id, $event)">
          <strong class="title">{{ titleWithCount(t) }}</strong>
          @if (t.body) {
            <div class="body">{{ t.body }}</div>
          }
          @if (note(); as n) {
            <div class="body err" role="alert">{{ n }}</div>
          }
          <div class="actions">
            @if (t.primary; as a) {
              <button type="button" class="btn primary" (click)="act(t, a)" data-testid="notice-toast-primary">{{ actionLabel(a) }}</button>
            }
            <!-- A question with three answers ("Update", "Later", "Skip this version") shows all of them. -->
            @if (t.tertiary; as c) {
              @if (t.secondary; as b) {
                <button type="button" class="btn" (click)="act(t, b)" data-testid="notice-toast-secondary">{{ actionLabel(b) }}</button>
              }
              <button type="button" class="btn" (click)="act(t, c)" data-testid="notice-toast-tertiary">{{ actionLabel(c) }}</button>
            }
            @if (t.id > 0) {
              <button type="button" class="btn ghost" (click)="show(t)" data-testid="notice-toast-show" i18n="@@notifications.toasts.show">Show in Notifications</button>
            }
          </div>
          <button type="button" class="x" aria-label="Dismiss notification" i18n-aria-label="@@notifications.toasts.dismiss" (click)="store.dismissToast(t.id)"><imt-icon name="close" [size]="12" /></button>
        </div>
      }
    </div>
  `,
  styles: `
    :host { position: fixed; left: 64px; bottom: 32px; z-index: 1900; pointer-events: none; }
    .stack { display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
    .toast { pointer-events: auto; position: relative; width: 360px; max-width: calc(100vw - 96px); padding: 10px 32px 10px 12px; border: 1px solid var(--border-2); border-left: 3px solid var(--accent); border-radius: 8px; background: var(--bg-3); color: var(--text); box-shadow: var(--shadow); display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-md); }
    .toast[data-severity='error'] { border-left-color: var(--lvl-error); }
    .toast[data-severity='warning'] { border-left-color: var(--lvl-warn); }
    .toast[data-severity='success'] { border-left-color: var(--ok); }
    .title { overflow-wrap: anywhere; }
    .body { color: var(--text-2); line-height: 1.4; overflow-wrap: anywhere; }
    .err { color: var(--danger-text, var(--lvl-error)); }
    .actions { display: flex; gap: 6px; flex-wrap: wrap; }
    .x { all: unset; position: absolute; top: 6px; right: 8px; cursor: pointer; color: var(--text-2); line-height: 1; }
    .x:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  `,
})
export class NotificationToasts {
  protected readonly store = inject(NotificationsStore);
  private readonly actions = inject(NoticeActions);
  private readonly ctx = inject(COMMAND_CONTEXT);
  protected readonly actionLabel = actionLabel;
  protected readonly titleWithCount = titleWithCount;
  protected readonly note = signal<string | null>(null);

  protected async act(t: Notice, a: NoticeAction): Promise<void> {
    const err = await this.actions.run(t, a);
    this.note.set(err);
    if (!err) this.store.dismissToast(t.id);
  }

  protected show(t: Notice): void {
    this.store.dismissToast(t.id);
    this.store.setFocus(t.id);
    this.ctx.showView(NOTIFICATIONS_VIEW);
  }

  /** The pointer or focus left a toast: once neither is on it, its timer runs again. */
  protected leave(id: number, e: MouseEvent | FocusEvent): void {
    const toast = e.currentTarget as HTMLElement;
    const focusStays = e instanceof FocusEvent ? toast.contains(e.relatedTarget as Node | null) : toast.contains(document.activeElement);
    const pointerStays = e instanceof FocusEvent && toast.matches(':hover');
    if (!focusStays && !pointerStays) this.store.releaseToast(id);
  }
}
