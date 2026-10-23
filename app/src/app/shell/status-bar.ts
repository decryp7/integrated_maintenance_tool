import { CdkConnectedOverlay, CdkOverlayOrigin, ConnectedPosition } from '@angular/cdk/overlay';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { formatCount, tzLabel } from '../core/format';
import { formatDuration } from '../shared/format/units';
import { WorkspaceStatusStore } from '../core/workspace/status.store';
import { WorkspaceStore } from '../core/workspace.store';
import { LiveTail } from '../workspace/live-tail';
import { BackgroundWork } from '../windowing/background-work';
import { ImportStatus } from '../import/import-ui';
import { StatusJobs } from '../jobs/status-jobs';
import { ProblemsBadge } from '../problems/problems-badge';
import { AiBrowsingChip } from '../browser/ai-browsing-chip';
import { ServiceStatusChip } from '../health/service-status-chip';
import { LifecycleService } from '../lifecycle/lifecycle.service';

@Component({
  selector: 'debrief-status-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CdkConnectedOverlay, CdkOverlayOrigin, BackgroundWork, ImportStatus, StatusJobs, ProblemsBadge, AiBrowsingChip, ServiceStatusChip],
  template: `
    @if (store.summary(); as s) {
      @if (s.sources.length) {
      <span class="keep strong" [title]="indexedTitle(s.loadMs)" i18n="@@shell.statusBar.events">{{ fmt(s.total) }} events</span>
      @if (store.matchTotal() !== null) {
        <span class="keep" i18n="@@shell.statusBar.matching">{{ fmt(store.matchTotal()!) }} matching</span>
      }
      <span class="low" i18n="@@shell.statusBar.instances">{{ fmt(store.instanceCount()) }} instances</span>
      @if (s.profileName) {
        <span class="low" data-testid="status-profile" [title]="profileHint(s.profileName)" i18n="@@shell.statusBar.profile">Profile: {{ s.profileName }}</span>
      }
      } @else {
        <span i18n="@@shell.statusBar.noFiles">No files open</span>
      }
    } @else {
      <span i18n="@@shell.statusBar.noWorkspace">No workspace</span>
    }
    <span class="spacer"></span>
    <debrief-service-status-chip class="keep" />
    @if (otherBusy(); as busy) {
      <span class="busy keep" role="status" data-testid="status-busy"><span class="spinner" aria-hidden="true"></span>{{ busy }}</span>
    }
    <debrief-import-status />
    <debrief-ai-browsing-chip />
    <button
      type="button"
      class="live"
      data-testid="live-toggle"
      aria-label="Live tail"
      i18n-aria-label="@@shell.statusBar.liveTail"
      [class.on]="live.enabled()"
      [class.failed]="!!live.error()"
      [attr.aria-pressed]="live.enabled()"
      [disabled]="!live.available()"
      [title]="liveTitle()"
      (click)="live.toggle()"
      i18n="@@shell.statusBar.live"
    >
      <span class="live-dot" aria-hidden="true"></span>Live
    </button>
    <debrief-background-work />
    <debrief-problems-badge />
    <debrief-status-jobs />
    <button class="tz" (click)="store.setTimezone(store.timezone() === 'utc' ? 'local' : 'utc')" title="Toggle UTC / local time" i18n-title="@@shell.statusBar.toggleTimezone">{{ tz() }}</button>
    <button type="button" class="tz details" data-testid="status-details" cdkOverlayOrigin #origin="cdkOverlayOrigin" aria-haspopup="dialog" [attr.aria-expanded]="detailsOpen()" (click)="detailsOpen.set(!detailsOpen())" i18n="@@shell.statusBar.details">Details</button>
    <ng-template cdkConnectedOverlay [cdkConnectedOverlayOrigin]="origin" [cdkConnectedOverlayOpen]="detailsOpen()" [cdkConnectedOverlayPositions]="positions" cdkConnectedOverlayHasBackdrop cdkConnectedOverlayBackdropClass="cdk-overlay-transparent-backdrop" (backdropClick)="detailsOpen.set(false)" (detach)="detailsOpen.set(false)" (overlayKeydown)="$event.key === 'Escape' && detailsOpen.set(false)">
      <dl class="details-pop" role="dialog" aria-label="Details" i18n-aria-label="@@shell.statusBar.detailsLabel" data-testid="status-details-popover">
        @if (store.summary(); as s) {
          <div><dt i18n="@@shell.statusBar.detail.sources">Sources</dt><dd>{{ fmt(s.sources.length) }}</dd></div>
          <div><dt i18n="@@shell.statusBar.detail.events">Events</dt><dd>{{ fmt(s.total) }}</dd></div>
          @if (s.untimed > 0) {
            <div data-testid="status-untimed" title="Events without a timestamp: in the grid, not on the timeline" i18n-title="@@shell.statusBar.untimedTitle"><dt i18n="@@shell.statusBar.detail.untimed">Without a time</dt><dd>{{ fmt(s.untimed) }}</dd></div>
          }
          @if (s.dropped > 0) {
            <div title="Events removed by the profile pipeline" i18n-title="@@shell.statusBar.droppedTitle"><dt i18n="@@shell.statusBar.detail.dropped">Dropped by pipeline</dt><dd>{{ fmt(s.dropped) }}</dd></div>
          }
          <div><dt i18n="@@shell.statusBar.detail.group">Group</dt><dd>{{ store.effectiveGroupBy().join(', ') || '—' }}</dd></div>
        }
        <div><dt i18n="@@shell.statusBar.detail.importers">Importers</dt><dd>{{ importers() }}</dd></div>
        <div><dt i18n="@@shell.statusBar.detail.version">Version</dt><dd>Debrief {{ version() }}</dd></div>
      </dl>
    </ng-template>
  `,
  styles: `
    :host { height: var(--status-h); flex: none; display: flex; align-items: center; gap: 16px; padding: 0 12px; background: var(--statusbar-bg); color: var(--statusbar-text); border-top: 1px solid var(--border); font-size: var(--fs-sm); white-space: nowrap; overflow: hidden; }
    /* When space runs out the secondary labels truncate; the sources, events and matching counts and the controls
       (import Cancel, Live, time zone) keep their size. */
    :host > span:not(.spacer):not(.keep):not(.strong) { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
    .keep, .strong { flex: none; }
    /* Narrow windows (H1): the low-priority items go; the counts and the controls stay. */
    @media (max-width: 1000px) { .low { display: none; } }
    .strong { font-weight: 600; color: var(--statusbar-strong); }
    .spacer { flex: 1; }
    .busy { display: inline-flex; align-items: center; gap: 6px; }
    .spinner { width: 9px; height: 9px; border-radius: 50%; border: 2px solid var(--border-3); border-top-color: var(--accent); animation: spin 0.8s linear infinite; }
    @keyframes spin { to { transform: rotate(360deg); } }
    @media (prefers-reduced-motion: reduce) { .spinner { animation: none; } }
    .tz { border: 0; background: transparent; color: inherit; cursor: pointer; padding: 0; font-size: var(--fs-sm); }
    .tz:hover { text-decoration: underline; }
    .details-pop { margin: 0; min-width: 240px; padding: 8px 12px; border: 1px solid var(--border-3); border-radius: var(--r-3); background: var(--bg-3); color: var(--text); box-shadow: var(--shadow-pop); font-size: var(--fs-sm); display: flex; flex-direction: column; gap: 4px; }
    .details-pop > div { display: flex; justify-content: space-between; gap: 16px; }
    .details-pop dt { color: var(--muted); }
    .details-pop dd { margin: 0; font-variant-numeric: tabular-nums; text-align: end; overflow-wrap: anywhere; }
    .live { display: inline-flex; align-items: center; gap: 6px; height: 18px; padding: 0 8px; border-radius: 9px; border: 1px solid currentColor; background: transparent; color: inherit; cursor: pointer; font-size: var(--fs-xs); font-weight: 600; }
    .live:hover:not(:disabled) { background: var(--bg-hover); color: var(--statusbar-strong); }
    .live:disabled { cursor: default; opacity: 0.5; }
    .live-dot { width: 7px; height: 7px; border-radius: 50%; border: 1px solid currentColor; }
    .live.on, .live.on:hover { background: var(--accent); color: var(--accent-ink); border-color: var(--accent); }
    .live.on .live-dot { background: currentColor; animation: live-pulse 1.6s ease-in-out infinite; }
    .live.failed { border-style: dashed; }
    @keyframes live-pulse { 50% { opacity: 0.35; } }
    @media (prefers-reduced-motion: reduce) { .live.on .live-dot { animation: none; } }
  `,
})
export class StatusBar {
  protected readonly store = inject(WorkspaceStore);
  protected readonly live = inject(LiveTail);
  /** The running app's version, as the backend reports it. */
  protected readonly version = computed(() => this.lifecycle.status()?.version ?? '');
  private readonly lifecycle = inject(LifecycleService);
  protected readonly liveTitle = computed(() => {
    const error = this.live.error();
    if (error) return $localize`:@@shell.statusBar.liveFailed:Live tail failed: ${error}:error:`;
    if (this.live.enabled()) return $localize`:@@shell.statusBar.liveOn:Live tail is on: new lines appear as the files grow. Click to stop.`;
    return this.live.available()
      ? $localize`:@@shell.statusBar.liveFollow:Follow the opened files as they grow`
      : $localize`:@@shell.statusBar.liveOpenFiles:Open files to follow them live`;
  });
  protected readonly fmt = formatCount;
  protected readonly detailsOpen = signal(false);
  protected readonly positions: ConnectedPosition[] = [{ originX: 'end', originY: 'top', overlayX: 'end', overlayY: 'bottom', offsetY: -4 }];
  protected profileHint(name: string): string {
    return $localize`:@@shell.statusBar.profileTitle:The parsing profile in use: ${name}:name:`;
  }
  private readonly status = inject(WorkspaceStatusStore);
  /** Work other than an import (its own item shows imports): "Applying the profile…", "Clearing workspace…". */
  protected readonly otherBusy = computed(() => (this.status.importing() ? null : this.status.busyLabel()));
  protected indexedTitle(loadMs: number): string {
    const took = formatDuration(loadMs);
    return $localize`:@@shell.statusBar.indexedTitle:Indexed in ${took}:duration:`;
  }
  protected readonly tz = computed(() => tzLabel(this.store.timezone()));
  protected readonly importers = computed(() => {
    const ids = [...new Set((this.store.summary()?.sources ?? []).map((s) => s.importer))];
    return ids.length ? ids.join(' · ') : '—';
  });
}
