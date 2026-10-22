import { formatBytes, formatExactBytes, formatRelativeTime } from '../shared/format/units';
import { ChangeDetectionStrategy, Component, computed, DestroyRef, effect, inject, signal, untracked } from '@angular/core';
import { TailFileStatus } from '../core/api.types';
import { formatCount } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { LiveTail } from '../workspace/live-tail';
import { SettingsDialogService } from '../settings/settings-dialog';
import { LIVE_TAIL_SECTION } from '../settings/settings-model';
import { stateHint, stateLabel, TailStatusStore } from './tail-status.store';
import { NextSteps } from '../ui/next-steps';
import { localizeDecimal } from '../i18n/locale';

/** The status table refreshes this often while the panel is open and live tail runs. */
export const STATUS_POLL_MS = 2000;

/** Dock view: per-file live tail state and a read-only summary of the live tail options (changed in Settings ▸ Live tail). */
@Component({
  selector: 'imt-tail-status-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [NextSteps],
  template: `
    <div class="controls">
      <button
        type="button"
        class="btn"
        [class.primary]="live.enabled()"
        [attr.aria-pressed]="live.enabled()"
        [disabled]="!live.available()"
        (click)="live.toggle()"
      >{{ toggleLabel() }}</button>
      <p class="summary" data-testid="tail-summary">{{ summary() }}</p>
      <button type="button" class="btn" data-testid="tail-settings" (click)="openSettings()" i18n="@@live.tailStatus.settings">Settings…</button>
    </div>
    @if (live.error(); as e) { <div class="err" role="alert">{{ e }}</div> }

    @if (status.status(); as st) {
      <div class="totals" role="status" aria-live="off">
        <span i18n="@@live.tailStatus.totalFiles"><strong>{{ fmt(st.totals.files) }}</strong> files</span>
        <span i18n="@@live.tailStatus.totalTailing"><strong>{{ st.totals.tailing }}</strong> tailing</span>
        @if (st.totals.errors) { <span class="bad" i18n="@@live.tailStatus.totalErrors"><strong>{{ st.totals.errors }}</strong> with errors</span> }
        <span [title]="exact(st.totals.bytes)">{{ bytes(st.totals.bytes) }}</span>
        <span i18n="@@live.tailStatus.totalRate"><strong>{{ rate(st.totals.linesPerSec) }}</strong> lines/s</span>
        <span i18n="@@live.tailStatus.totalEvents">{{ count(st.totals.records) }} events</span>
        <span title="Events removed by the retention limit since the workspace was opened" i18n-title="@@live.tailStatus.evictedTitle" i18n="@@live.tailStatus.totalEvicted">{{ count(st.totals.evicted) }} evicted</span>
      </div>
      <div class="list" tabindex="0" role="region" aria-label="Tailed files" i18n-aria-label="@@live.tailStatus.filesAria">
        <table>
          <thead>
            <tr>
              <th scope="col" i18n="@@live.tailStatus.colFile">File</th><th scope="col" i18n="@@live.tailStatus.colGroup">Group</th><th scope="col" i18n="@@live.tailStatus.colMode">Mode</th><th scope="col" class="r" i18n="@@live.tailStatus.colOffset">Offset / size</th>
              <th scope="col" class="r" i18n="@@live.tailStatus.colRate">Lines/s</th><th scope="col" i18n="@@live.tailStatus.colLast">Last change</th><th scope="col" i18n="@@live.tailStatus.colState">State</th>
            </tr>
          </thead>
          <tbody>
            @for (f of st.files; track f.sourceId) {
              <tr data-testid="tail-file">
                <td class="mono" [title]="f.path">{{ f.name }}</td>
                <td class="mono faint" [title]="f.group">{{ shortGroup(f) }}</td>
                <td>{{ modeLabel(f) }}</td>
                <td class="mono r" [title]="exact(f.offset) + ' / ' + exact(f.size)">{{ bytes(f.offset) }} / {{ bytes(f.size) }}</td>
                <td class="mono r">{{ rate(f.linesPerSec) }}</td>
                <td class="nowrap">{{ ago(f.lastEvent) }}</td>
                <td [title]="hint(f)"><span class="state" [class]="f.state.kind">{{ label(f) }}</span>@if (f.state.kind === 'error') { <span class="err-msg">{{ errorOf(f) }}</span> }</td>
              </tr>
            } @empty {
              @if (sourcesOpen()) {
                <tr><td colspan="7" class="faint" i18n="@@live.tailStatus.noFileState">The state of each tailed file is not available yet. Live tail still adds new lines.</td></tr>
              } @else {
                <tr><td colspan="7" class="faint" i18n="@@live.tailStatus.noFiles">No files are open. <imt-next-steps /></td></tr>
              }
            }
          </tbody>
        </table>
      </div>
    } @else if (status.error(); as e) {
      <div class="empty-state"><div class="err" i18n="@@live.tailStatus.loadFailed">The tail status could not be loaded: {{ e }}</div><button type="button" class="btn" (click)="status.refresh()" i18n="@@live.tailStatus.retry">Retry</button></div>
    } @else {
      <div class="empty-state faint" role="status" i18n="@@live.tailStatus.loading">Loading…</div>
    }
  `,
  styles: `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .controls { flex: none; display: flex; flex-wrap: wrap; align-items: center; gap: 10px; padding: 10px 12px; border-bottom: 1px solid var(--border); }
    .summary { flex: 1; min-width: 200px; margin: 0; font-size: var(--fs-sm); color: var(--text-2); }
    .err { color: var(--danger-text); padding: 6px 12px; font-size: var(--fs-sm); }
    .totals { flex: none; display: flex; flex-wrap: wrap; gap: 4px 16px; padding: 6px 12px; font-size: var(--fs-sm); color: var(--text-2); border-bottom: 1px solid var(--border); }
    .totals .bad { color: var(--danger-text); }
    .list { flex: 1; min-height: 0; overflow: auto; }
    table { width: 100%; border-collapse: collapse; font-size: var(--fs-sm); }
    th { position: sticky; top: 0; background: var(--bg-2); text-align: left; font-weight: 600; color: var(--muted); padding: 5px 8px; border-bottom: 1px solid var(--border); white-space: nowrap; }
    td { padding: 4px 8px; border-bottom: 1px solid var(--border); color: var(--text-2); max-width: 280px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .r { text-align: right; }
    .nowrap { white-space: nowrap; }
    .state { display: inline-block; padding: 0 6px; border-radius: 8px; border: 1px solid var(--border-2); font-size: var(--fs-xs); }
    .state.rotated, .state.truncated { border-color: var(--warn-text); color: var(--warn-text); }
    .state.deleted { border-style: dashed; }
    .state.error { border-color: var(--danger-text); color: var(--danger-text); }
    .err-msg { margin-left: 6px; color: var(--danger-text); }
  `,
})
export class TailStatusView {
  protected readonly fmt = formatCount;
  protected readonly live = inject(LiveTail);
  protected readonly status = inject(TailStatusStore);
  private readonly workspace = inject(WorkspaceStore);
  /** Files are open although the engine lists none: it keeps no per-file tail state (yet). */
  protected readonly sourcesOpen = computed(() => (this.workspace.summary()?.sources.length ?? 0) > 0);
  private readonly settings = inject(SettingsDialogService);
  private readonly watchAuto = $localize`:@@live.tailStatus.watchAuto.short:auto`;
  private readonly watchEvents = $localize`:@@live.tailStatus.watchEvents.short:file events`;
  private readonly watchPolling = $localize`:@@live.tailStatus.watchPolling.short:polling`;
  protected readonly opts = computed(() => this.live.options());
  private readonly now = signal(Date.now());

  protected readonly toggleLabel = computed(() => (this.live.enabled() ? $localize`:@@live.tailStatus.liveOn:Live tail on` : $localize`:@@live.tailStatus.liveOff:Live tail off`));
  protected readonly count = formatCount;
  protected readonly bytes = formatBytes;
  protected readonly exact = formatExactBytes;
  protected readonly label = (f: TailFileStatus) => stateLabel(f.state);
  protected readonly hint = (f: TailFileStatus) => stateHint(f.state);

  constructor() {
    void this.status.refresh();
    let timer: ReturnType<typeof setInterval> | undefined;
    effect(() => {
      const on = this.live.enabled();
      untracked(() => {
        clearInterval(timer);
        timer = on
          ? setInterval(() => {
              this.now.set(Date.now());
              void this.status.refresh();
            }, STATUS_POLL_MS)
          : undefined;
      });
    });
    inject(DestroyRef).onDestroy(() => clearInterval(timer));
  }

  protected ago(ts: number | null): string {
    return formatRelativeTime(ts, this.now());
  }

  protected rate(v: number): string {
    return v >= 100 ? Math.round(v).toString() : localizeDecimal(v.toFixed(1));
  }

  protected modeLabel(f: TailFileStatus): string {
    if (f.state.kind === 'whole') return $localize`:@@live.tailStatus.modeWhole:Whole file`;
    if (f.mode === 'events') return $localize`:@@live.tailStatus.modeEvents:Events`;
    if (f.mode === 'polling') return $localize`:@@live.tailStatus.modePolling:Polling`;
    return $localize`:@@live.tailStatus.modeOff:Off`;
  }

  protected shortGroup(f: TailFileStatus): string {
    return f.group.split(/[\\/]/).filter(Boolean).slice(-1)[0] ?? f.group;
  }

  protected errorOf(f: TailFileStatus): string {
    return f.state.kind === 'error' ? f.state.message : '';
  }

  /** One line with the options in force; they are edited in Settings ▸ Live tail. */
  protected readonly summary = computed(() => {
    const o = this.opts();
    const watch = o.watch === 'events' ? this.watchEvents : o.watch === 'polling' ? this.watchPolling : this.watchAuto;
    const parts = [$localize`:@@live.tailStatus.summary.watch:Watch: ${watch}:mode:`];
    if (o.watch !== 'events') parts.push($localize`:@@live.tailStatus.summary.poll:poll every ${o.pollIntervalMs ?? 2000}:ms: ms`);
    const max = o.retention?.maxRecords;
    parts.push(max ? $localize`:@@live.tailStatus.summary.keep:keep ${formatCount(max)}:n: events` : $localize`:@@live.tailStatus.summary.keepAll:keep all events`);
    return parts.join(' · ');
  });

  protected openSettings(): void {
    this.settings.open(LIVE_TAIL_SECTION);
  }
}
