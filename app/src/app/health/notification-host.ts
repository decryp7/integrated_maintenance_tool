import { ChangeDetectionStrategy, Component, computed, DestroyRef, effect, inject, signal } from '@angular/core';
import { Icon } from '../shared/icon';
import { HostAction, HostNotification, NOTIFICATION_SOURCES, persists, TOAST_MS, visibleNotifications } from './notification-host.model';

/**
 * The one place notifications appear (ADR 0055), replacing a separate toast component per feature. One toast per cause,
 * details collapsed behind "Details", updated in place; errors and critical failures stay until resolved or dismissed;
 * a cause the status chip states (`health:...`) is never toasted. The host owns the time of every toast (sources only post, update
 * and resolve): a toast that times out stays `TOAST_MS`, is held while the pointer or focus is on it, and starts over when its
 * cause is posted again. Mounted once per window.
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
          data-testid="notification"
          [attr.role]="persistent(n) ? 'alert' : 'status'"
          (mouseenter)="hold(n)"
          (focusin)="hold(n)"
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
                <button type="button" class="btn" [class.primary]="a.kind === 'primary'" [class.ghost]="a.kind === 'ghost'" [attr.data-testid]="a.id ? 'notification-action-' + a.id : 'notification-action'" (click)="run(n, a)">{{ a.label }}</button>
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
    return persists(n);
  }

  /** Per cause: the occurrence the running timer belongs to, and the timer. */
  private readonly timers = new Map<string, { key: string; handle: ReturnType<typeof setTimeout> | undefined }>();
  /** Causes the pointer or focus is on. */
  private readonly held = new Set<string>();

  constructor() {
    // Timers are outside the signal graph: one per toast that times out, started when it appears or is posted again.
    effect(() => this.sync(this.items()));
    inject(DestroyRef).onDestroy(() => this.timers.forEach((t) => clearTimeout(t.handle)));
  }

  private sync(items: readonly HostNotification[]): void {
    const live = new Set(items.map((n) => n.cause));
    for (const [cause, t] of this.timers) {
      if (live.has(cause)) continue;
      clearTimeout(t.handle);
      this.timers.delete(cause);
      this.held.delete(cause);
    }
    for (const n of items) {
      if (persists(n)) continue;
      const key = `${n.instance ?? ''}#${n.stamp ?? n.count ?? ''}`;
      const current = this.timers.get(n.cause);
      if (current?.key === key) continue;
      clearTimeout(current?.handle);
      this.timers.set(n.cause, { key, handle: this.held.has(n.cause) ? undefined : this.start(n) });
    }
  }

  private start(n: HostNotification): ReturnType<typeof setTimeout> {
    return setTimeout(() => n.dismiss(), n.ttl ?? TOAST_MS);
  }

  /** The pointer or focus is on a toast: its time stops. */
  protected hold(n: HostNotification): void {
    this.held.add(n.cause);
    const t = this.timers.get(n.cause);
    if (t) {
      clearTimeout(t.handle);
      t.handle = undefined;
    }
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
    if (focusStays || pointerStays) return;
    this.held.delete(n.cause);
    const t = this.timers.get(n.cause);
    if (t && !persists(n)) {
      clearTimeout(t.handle);
      t.handle = this.start(n);
    }
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
