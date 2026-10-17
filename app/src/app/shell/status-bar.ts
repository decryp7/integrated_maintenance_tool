import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { formatCount, tzLabel } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { LiveTail } from '../workspace/live-tail';
import { BackgroundWork } from '../windowing/background-work';

@Component({
  selector: 'imt-status-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [BackgroundWork],
  template: `
    @if (store.summary(); as s) {
      <span class="strong">{{ s.sources.length }} source{{ s.sources.length === 1 ? '' : 's' }}</span>
      <span>{{ fmt(s.total) }} events</span>
      @if (store.matchTotal() !== null) {
        <span>{{ fmt(store.matchTotal()!) }} matching</span>
      }
      <span>{{ store.instanceCount() }} instances</span>
      @if (s.dropped > 0) {
        <span class="dropped" title="Events removed by the profile pipeline">{{ fmt(s.dropped) }} dropped by pipeline</span>
      }
      <span>Group: {{ store.effectiveGroupBy().join(', ') || '—' }}</span>
    } @else {
      <span>No workspace</span>
    }
    <span class="spacer"></span>
    <button
      type="button"
      class="live"
      data-testid="live-toggle"
      aria-label="Live tail"
      [class.on]="live.enabled()"
      [class.failed]="!!live.error()"
      [attr.aria-pressed]="live.enabled()"
      [disabled]="!live.available()"
      [title]="liveTitle()"
      (click)="live.toggle()"
    >
      <span class="live-dot" aria-hidden="true"></span>Live
    </button>
    <imt-background-work />
    <span>Importers: {{ importers() }}</span>
    <button class="tz" (click)="store.setTimezone(store.timezone() === 'utc' ? 'local' : 'utc')" title="Toggle UTC / local time">{{ tz() }}</button>
    <span>IMT 0.1.0</span>
  `,
  styles: `
    :host { height: 24px; flex: none; display: flex; align-items: center; gap: 16px; padding: 0 12px; background: var(--accent); color: var(--accent-ink); font-size: 12px; white-space: nowrap; overflow: hidden; }
    .strong { font-weight: 600; }
    .spacer { flex: 1; }
    .tz { border: 0; background: transparent; color: inherit; cursor: pointer; padding: 0; font-size: 12px; }
    .tz:hover { text-decoration: underline; }
    .live { display: inline-flex; align-items: center; gap: 6px; height: 18px; padding: 0 8px; border-radius: 9px; border: 1px solid currentColor; background: transparent; color: inherit; cursor: pointer; font-size: 11px; font-weight: 600; }
    .live:hover:not(:disabled) { background: color-mix(in srgb, var(--accent-ink) 18%, transparent); }
    .live:disabled { cursor: default; opacity: 0.5; }
    .live-dot { width: 7px; height: 7px; border-radius: 50%; border: 1px solid currentColor; }
    .live.on, .live.on:hover { background: var(--accent-ink); color: var(--accent); border-color: var(--accent-ink); }
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
    if (error) return `Live tail failed: ${error}`;
    if (this.live.enabled()) return 'Live tail is on: new lines appear as the files grow. Click to stop.';
    return this.live.available() ? 'Follow the opened files as they grow' : 'Open files to follow them live';
  });
  protected readonly fmt = formatCount;
  protected readonly tz = computed(() => tzLabel(this.store.timezone()));
  protected readonly importers = computed(() => {
    const ids = [...new Set((this.store.summary()?.sources ?? []).map((s) => s.importer))];
    return ids.length ? ids.join(' · ') : '—';
  });
}
