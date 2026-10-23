import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, signal, untracked } from '@angular/core';
import { formatDateTime } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { JobsStore } from '../jobs/jobs.store';
import { actionAvailable, actionLabel, jobClearedNote, listedJobs, NoticeActions } from './notice-actions';
import { Icon } from '../shared/icon';
import { PanelEmpty } from '../jobs/panel-empty';
import { KindFilter, KindOption } from '../ui/kind-filter';
import { NotificationsStore } from './notifications.store';
import { Notice, NOTICE_SOURCES, NoticeAction } from './notifications.types';
import { sourceLabel, titleWithCount } from './notifications-text';

/**
 * Notifications: what happened and may need attention (long jobs finished or failed, live alerts, files not
 * imported, quarantined plugins, the AI paused, updates, a recovered session), grouped by source, with each
 * message's action, Dismiss and Mark all read. The engine keeps the list (dedupe with counts, caps, read state).
 */
@Component({
  selector: 'debrief-notifications-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [KindFilter, Icon, PanelEmpty],
  template: `
    <div class="bar" role="toolbar" aria-label="Notification filters" i18n-aria-label="@@notifications.panel.filters">
      <div class="seg" role="group" aria-label="Show" i18n-aria-label="@@notifications.panel.show" data-testid="notices-view">
        <button type="button" class="btn small" [attr.aria-pressed]="store.read() === 'all'" (click)="store.setRead('all')" data-testid="notices-view-all" i18n="@@notifications.panel.view.all">All</button>
        <button type="button" class="btn small" [attr.aria-pressed]="store.read() === 'unread'" (click)="store.setRead('unread')" data-testid="notices-unread-only" i18n="@@notifications.panel.view.unread">Unread</button>
      </div>
      <debrief-kind-filter label="Source" i18n-label="@@notifications.panel.source" [options]="sourceOptions()" [selected]="store.sources()" (toggled)="store.toggleSource($any($event))" (cleared)="clearSources()" />
      <span class="spacer"></span>
      <button type="button" class="btn ghost small" [disabled]="!store.unread()" (click)="store.markAllRead()" data-testid="notices-mark-all" i18n="@@notifications.panel.markAll">Mark all read</button>
    </div>
    @if (store.error(); as e) {
      <div class="err" role="alert">
        <span class="grow">{{ e }}</span>
        <button type="button" class="btn small" (click)="store.retryRequest()" i18n="@@notifications.panel.retryRequest">Retry</button>
      </div>
    }
    @if (note(); as n) {
      <p class="err" role="alert">{{ n }}</p>
    }
    <p class="sr-only" aria-live="polite" data-testid="notices-live">{{ announcement() }}</p>
    @if (!store.notices().length) {
      <debrief-panel-empty class="empty" data-testid="notices-empty" icon="bell" title="Nothing needs your attention" i18n-title="@@notifications.panel.empty.title"><ng-container i18n="@@notifications.panel.empty">Finished jobs, alerts and problems with your files show up here.</ng-container></debrief-panel-empty>
    }
    @for (g of store.groups(); track g.source) {
      <h3 [id]="'notices-h-' + g.source">{{ sourceLabel(g.source) }}</h3>
      <ul role="list" [attr.aria-labelledby]="'notices-h-' + g.source">
        @for (n of g.notices; track n.id) {
          <li role="listitem" class="notice" [class.unread]="!n.read" [class.focus]="store.focus() === n.id" [attr.data-severity]="n.severity" [attr.data-testid]="'notice-' + n.id" [attr.aria-label]="rowLabel(n)">
            <div class="row">
              <span class="dot" aria-hidden="true"></span>
              <strong class="title">{{ titleWithCount(n) }}</strong>
              <span class="muted small time">{{ time(n.at) }}</span>
              <span class="row-actions">
                @if (!n.read) {
                  <button type="button" class="icon-btn" (click)="store.markRead(n.id)" [attr.aria-label]="markReadLabel(n)" [title]="markReadLabel(n)" [attr.data-testid]="'notice-read-' + n.id"><debrief-icon name="check" [size]="14" /></button>
                }
                <button type="button" class="icon-btn" (click)="store.dismiss(n.id)" [attr.aria-label]="dismissLabel(n)" [title]="dismissLabel(n)" [attr.data-testid]="'notice-dismiss-' + n.id"><debrief-icon name="close" [size]="14" /></button>
              </span>
            </div>
            @if (n.body) {
              <p class="body debrief-selectable">{{ n.body }}</p>
            }
            @if (cleared(n)) {
              <p class="muted small cleared" [attr.data-testid]="'notice-cleared-' + n.id">{{ clearedNote }}</p>
            }
            <div class="actions">
              @if (store.localAction(n.id); as label) {
                <button type="button" class="btn small" (click)="store.runLocal(n.id)" [attr.data-testid]="'notice-primary-' + n.id">{{ label }}</button>
              }
              @if (n.primary; as a) {
                @if (available(a)) {
                  <button type="button" class="btn small" (click)="act(n, a)" [attr.data-testid]="'notice-primary-' + n.id">{{ actionLabel(a) }}</button>
                }
              }
              @if (n.secondary; as a) {
                @if (available(a)) {
                  <button type="button" class="btn small" (click)="act(n, a)">{{ actionLabel(a) }}</button>
                }
              }
              @if (n.tertiary; as a) {
                @if (available(a)) {
                  <button type="button" class="btn small" (click)="act(n, a)" [attr.data-testid]="'notice-tertiary-' + n.id">{{ actionLabel(a) }}</button>
                }
              }
            </div>
          </li>
        }
      </ul>
    }
  `,
  styles: `
    :host { position: relative; display: flex; flex-direction: column; gap: 6px; padding: 8px 10px; overflow: auto; height: 100%; box-sizing: border-box; font-size: var(--fs-md); }
    .bar { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; }
    .spacer, .grow { flex: 1; }
    .err { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: 6px; border: 1px solid var(--lvl-error); margin: 0; }
    h3 { margin: 8px 0 2px; font-size: var(--fs-sm); font-weight: 600; color: var(--text-2); }
    ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    .notice { border: 1px solid var(--border); border-left: 3px solid var(--border-2); border-radius: 6px; padding: 6px 8px; display: flex; flex-direction: column; gap: 4px; }
    .notice[data-severity='error'] { border-left-color: var(--lvl-error); }
    .notice[data-severity='warning'] { border-left-color: var(--lvl-warn); }
    .notice[data-severity='success'] { border-left-color: var(--ok); }
    .notice.focus { outline: 2px solid var(--focus-ring); }
    .row { display: flex; align-items: center; gap: 6px; min-width: 0; }
    .dot { width: 7px; height: 7px; border-radius: 50%; flex: none; }
    .unread .dot { background: var(--accent); }
    .title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; }
    .read .title, .notice:not(.unread) .title { font-weight: 400; color: var(--text-2); }
    .time { white-space: nowrap; }
    .body { margin: 0; color: var(--text-2); overflow-wrap: anywhere; }
    .actions { display: flex; flex-wrap: wrap; gap: 6px; justify-content: flex-end; }
    .row-actions { display: flex; gap: 2px; opacity: 0; }
    .notice:hover .row-actions, .notice:focus-within .row-actions { opacity: 1; }
    @media (hover: none) { .row-actions { opacity: 1; } }
    .cleared { margin: 0; }
    .small { font-size: var(--fs-xs); }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  `,
})
export class NotificationsPanel {
  protected readonly store = inject(NotificationsStore);
  private readonly actions = inject(NoticeActions);
  private readonly ws = inject(WorkspaceStore);
  private readonly jobs = inject(JobsStore);
  /** The listed jobs: actions of a job cleared from Jobs are hidden (its result is gone; the message keeps the outcome). */
  private readonly listed = computed(() => listedJobs(this.jobs));
  protected readonly clearedNote = jobClearedNote();
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  protected readonly sourceOptions = computed<KindOption[]>(() => {
    const counts = this.store.sourceCounts();
    return NOTICE_SOURCES.map((s) => ({ id: s, label: sourceLabel(s), count: counts.get(s) ?? 0 }));
  });

  protected clearSources(): void {
    for (const s of this.store.sources()) this.store.toggleSource(s);
  }
  protected readonly sourceLabel = sourceLabel;
  protected readonly actionLabel = actionLabel;
  protected readonly titleWithCount = titleWithCount;
  protected readonly note = signal<string | null>(null);
  protected readonly announcement = signal('');

  constructor() {
    let known = new Set<number>();
    effect(() => {
      const notices = this.store.snapshot().notices;
      untracked(() => {
        const fresh = notices.filter((n) => !known.has(n.id) && !n.read);
        const first = fresh[0];
        if (known.size && first) this.announcement.set(titleWithCount(first));
        known = new Set(notices.map((n) => n.id));
      });
    });
    // A clicked OS notification (or "Show in Notifications"): scroll the message into view and focus its button.
    effect(() => {
      const id = this.store.focus();
      if (id === null) return;
      untracked(() =>
        queueMicrotask(() => {
          const el = this.host.nativeElement.querySelector<HTMLElement>(`[data-testid="notice-${id}"]`);
          el?.scrollIntoView?.({ block: 'nearest' });
          el?.querySelector<HTMLElement>('button')?.focus();
        }),
      );
    });
  }

  protected time(at: number): string {
    return formatDateTime(at, this.ws.timezone());
  }

  protected rowLabel(n: Notice): string {
    const state = n.read ? $localize`:@@notifications.panel.read:read` : $localize`:@@notifications.panel.unread:unread`;
    return `${titleWithCount(n)}, ${state}`;
  }

  protected markReadLabel(n: Notice): string {
    return $localize`:@@notifications.panel.markReadLabel:Mark read: ${n.title}:title:`;
  }

  protected dismissLabel(n: Notice): string {
    return $localize`:@@notifications.panel.dismissLabel:Dismiss ${n.title}:title:`;
  }

  protected available(a: NoticeAction): boolean {
    return actionAvailable(a, this.listed());
  }

  /** Some action of the message was hidden because its job was cleared. */
  protected cleared(n: Notice): boolean {
    return [n.primary, n.secondary, n.tertiary].some((a) => a !== null && !this.available(a));
  }

  protected async act(n: Notice, a: NoticeAction): Promise<void> {
    this.note.set(await this.actions.run(n, a));
  }

}
