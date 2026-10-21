import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { formatCount, tzLabel } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { LiveTail } from '../workspace/live-tail';
import { BackgroundWork } from '../windowing/background-work';
import { ImportStatus } from '../import/import-ui';
import { StatusJobs } from '../jobs/status-jobs';

@Component({
  selector: 'imt-status-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [BackgroundWork, ImportStatus, StatusJobs],
  template: `
    @if (store.summary(); as s) {
      <span class="strong" i18n="@@shell.statusBar.sources">{s.sources.length, plural, =1 {{{ s.sources.length }} source} other {{{ s.sources.length }} sources}}</span>
      <span class="keep" i18n="@@shell.statusBar.events">{{ fmt(s.total) }} events</span>
      @if (store.matchTotal() !== null) {
        <span class="keep" i18n="@@shell.statusBar.matching">{{ fmt(store.matchTotal()!) }} matching</span>
      }
      <span i18n="@@shell.statusBar.instances">{{ fmt(store.instanceCount()) }} instances</span>
      @if (s.dropped > 0) {
        <span class="dropped" title="Events removed by the profile pipeline" i18n-title="@@shell.statusBar.droppedTitle" i18n="@@shell.statusBar.dropped">{{ fmt(s.dropped) }} dropped by pipeline</span>
      }
      <span i18n="@@shell.statusBar.group">Group: {{ store.effectiveGroupBy().join(', ') || '—' }}</span>
    } @else {
      <span i18n="@@shell.statusBar.noWorkspace">No workspace</span>
    }
    <span class="spacer"></span>
    <imt-import-status />
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
    <imt-background-work />
    <imt-status-jobs />
    <span i18n="@@shell.statusBar.importers">Importers: {{ importers() }}</span>
    <button class="tz" (click)="store.setTimezone(store.timezone() === 'utc' ? 'local' : 'utc')" title="Toggle UTC / local time" i18n-title="@@shell.statusBar.toggleTimezone">{{ tz() }}</button>
    <span>IMT 0.1.0</span>
  `,
  styles: `
    :host { height: var(--status-h); flex: none; display: flex; align-items: center; gap: 16px; padding: 0 12px; background: var(--statusbar-bg); color: var(--statusbar-text); border-top: 1px solid var(--border); font-size: var(--fs-sm); white-space: nowrap; overflow: hidden; }
    /* When space runs out the secondary labels truncate; the sources, events and matching counts and the controls
       (import Cancel, Live, time zone) keep their size. */
    :host > span:not(.spacer):not(.keep):not(.strong) { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
    .keep, .strong { flex: none; }
    .strong { font-weight: 600; color: var(--statusbar-strong); }
    .spacer { flex: 1; }
    .tz { border: 0; background: transparent; color: inherit; cursor: pointer; padding: 0; font-size: var(--fs-sm); }
    .tz:hover { text-decoration: underline; }
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
  protected readonly liveTitle = computed(() => {
    const error = this.live.error();
    if (error) return $localize`:@@shell.statusBar.liveFailed:Live tail failed: ${error}:error:`;
    if (this.live.enabled()) return $localize`:@@shell.statusBar.liveOn:Live tail is on: new lines appear as the files grow. Click to stop.`;
    return this.live.available()
      ? $localize`:@@shell.statusBar.liveFollow:Follow the opened files as they grow`
      : $localize`:@@shell.statusBar.liveOpenFiles:Open files to follow them live`;
  });
  protected readonly fmt = formatCount;
  protected readonly tz = computed(() => tzLabel(this.store.timezone()));
  protected readonly importers = computed(() => {
    const ids = [...new Set((this.store.summary()?.sources ?? []).map((s) => s.importer))];
    return ids.length ? ids.join(' · ') : '—';
  });
}
