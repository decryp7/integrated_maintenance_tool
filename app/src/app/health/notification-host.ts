import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { Icon } from '../shared/icon';
import { HostAction, HostNotification, NOTIFICATION_SOURCES, persists, visibleNotifications } from './notification-host.model';

/**
 * The one place notifications appear (ADR 0055), replacing a separate toast component per feature. One toast per cause,
 * details collapsed behind "Details", updated in place; errors and critical failures stay until resolved or dismissed;
 * a cause the status chip states (`health:...`) is never toasted. Mounted once per window.
 */
@Component({
  selector: 'debrief-notification-host',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  host: { 'data-snapshot-ignore': '' },
  template: `
    <div class="stack" role="region" aria-label="Notifications" i18n-aria-label="@@health.host.region" aria-live="polite">
      @for (n of items(); track n.cause) {
        <div
          class="toast"
          data-debrief-overlay
          data-toast
          [attr.data-severity]="n.severity"
          [attr.data-cause]="n.cause"
          [attr.data-testid]="n.testId ?? 'notification'"
          [attr.role]="persistent(n) ? 'alert' : 'status'"
          (mouseenter)="n.hold?.()"
          (focusin)="n.hold?.()"
          (mouseleave)="leave(n, $event)"
          (focusout)="leave(n, $event)"
        >
          <div class="head">
            <debrief-icon [name]="icon(n)" [size]="15" />
            <strong class="title">{{ n.title }}@if ((n.count ?? 1) > 1) {<span class="count" data-testid="notification-count"> (×{{ n.count }})</span>}</strong>
          </div>
          @if (n.body) {
            <div class="body debrief-selectable">{{ n.body }}</div>
          }
          @if (n.actions?.length || n.details) {
            <div class="actions">
              @for (a of n.actions; track $index) {
                <button type="button" class="btn" [class.primary]="a.kind === 'primary'" [class.ghost]="a.kind === 'ghost'" [attr.data-testid]="a.testId ?? 'notification-action'" (click)="run(n, a)">{{ a.label }}</button>
              }
              @if (n.details) {
                <button type="button" class="btn ghost" [attr.aria-expanded]="isOpen(n)" data-testid="notification-details-toggle" (click)="toggle(n)" i18n="@@health.host.details">Details</button>
              }
            </div>
          }
          @if (n.details && isOpen(n)) {
            <code class="details debrief-selectable" data-testid="notification-details">{{ n.details }}</code>
          }
          @if (failed()[n.cause]; as why) {
            <div class="body err" role="alert">{{ why }}</div>
          }
          <button type="button" class="x" aria-label="Dismiss notification" i18n-aria-label="@@health.host.dismiss" data-testid="notification-dismiss" (click)="n.dismiss()"><debrief-icon name="close" [size]="12" /></button>
        </div>
      }
    </div>
  `,
  styles: `
    :host { position: fixed; right: 16px; bottom: 32px; z-index: 1900; pointer-events: none; }
    .stack { display: flex; flex-direction: column; gap: 8px; align-items: flex-end; }
    .toast { pointer-events: auto; position: relative; width: 380px; max-width: calc(100vw - 32px); box-sizing: border-box; padding: 10px 32px 10px 12px; border: 1px solid var(--border-2); border-left: 3px solid var(--accent); border-radius: 8px; background: var(--bg-3); color: var(--text); box-shadow: var(--shadow); display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-md); }
    .toast[data-severity='success'] { border-left-color: var(--ok); }
    .toast[data-severity='warning'] { border-left-color: var(--lvl-warn); }
    .toast[data-severity='error'] { border-left-color: var(--lvl-error); }
    .toast[data-severity='critical'] { border-left-color: var(--lvl-fatal); }
    .head { display: flex; align-items: center; gap: 8px; min-width: 0; }
    .title { overflow-wrap: anywhere; }
    .count { color: var(--muted); font-weight: 400; font-size: var(--fs-xs); }
    .body { color: var(--text-2); line-height: 1.4; overflow-wrap: anywhere; }
    .err { color: var(--danger-text, var(--lvl-error)); }
    .actions { display: flex; gap: 6px; flex-wrap: wrap; }
    .details { display: block; max-height: 160px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--muted); font-size: var(--fs-xs); }
    .x { all: unset; position: absolute; top: 6px; right: 8px; cursor: pointer; color: var(--text-2); line-height: 1; }
    .x:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  `,
})
export class NotificationHost {
  private readonly sources = inject(NOTIFICATION_SOURCES, { optional: true }) ?? [];
  protected readonly items = computed(() => visibleNotifications(this.sources.flatMap((s) => s.items())));
  private readonly opened = signal<ReadonlySet<string>>(new Set());
  /** An action that rejected: why, until the next try. */
  protected readonly failed = signal<Record<string, string>>({});

  protected persistent(n: HostNotification): boolean {
    return persists(n.severity);
  }

  protected icon(n: HostNotification): string {
    return n.severity === 'success' ? 'check' : n.severity === 'info' ? 'bell' : 'alert';
  }

  private key(n: HostNotification): string {
    return `${n.cause}#${n.instance ?? ''}`;
  }

  protected isOpen(n: HostNotification): boolean {
    return this.opened().has(this.key(n));
  }

  protected toggle(n: HostNotification): void {
    const key = this.key(n);
    this.opened.update((s) => {
      const next = new Set(s);
      if (!next.delete(key)) next.add(key);
      return next;
    });
  }

  /** The pointer or focus left a toast: once neither is on it, its timer runs again. */
  protected leave(n: HostNotification, e: MouseEvent | FocusEvent): void {
    const toast = e.currentTarget as HTMLElement;
    const focusStays = e instanceof FocusEvent ? toast.contains(e.relatedTarget as Node | null) : toast.contains(document.activeElement);
    const pointerStays = e instanceof FocusEvent && toast.matches(':hover');
    if (!focusStays && !pointerStays) n.release?.();
  }

  /** Runs an action; the toast stays when it failed, with the reason. */
  protected async run(n: HostNotification, a: HostAction): Promise<void> {
    this.failed.update((f) => ({ ...f, [n.cause]: '' }));
    try {
      await a.run();
    } catch (e) {
      this.failed.update((f) => ({ ...f, [n.cause]: e instanceof Error ? e.message : String(e) }));
    }
  }
}
