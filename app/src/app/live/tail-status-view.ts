import { ChangeDetectionStrategy, ChangeDetectorRef, Component, computed, DestroyRef, effect, inject, signal, untracked } from '@angular/core';
import { DeletedFiles, LiveOptions, MAX_DETECT_EVERY_MS, TailFileStatus, WatchPreference } from '../core/api.types';
import { formatCount } from '../core/format';
import { LiveTail } from '../workspace/live-tail';
import { parseDetectEvery } from './live-ui-state';
import { stateHint, stateLabel, TailStatusStore } from './tail-status.store';
import { NextSteps } from '../ui/next-steps';
import { localizeDecimal } from '../i18n/locale';

/** The status table refreshes this often while the panel is open and live tail runs. */
export const STATUS_POLL_MS = 2000;

/** "1.2 MB" */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${localizeDecimal(v.toFixed(v < 10 ? 1 : 0))} ${units[i]}`;
}

/** "just now", "12 s ago", "3 min ago", "2 h ago" */
export function formatAgo(ts: number | null, now: number): string {
  if (ts === null) return '—';
  const s = Math.max(0, Math.round((now - ts) / 1000));
  if (s < 2) return $localize`:@@live.tailStatus.justNow:just now`;
  if (s < 60) return $localize`:@@live.tailStatus.secondsAgo:${s}:seconds: s ago`;
  if (s < 3600) return $localize`:@@live.tailStatus.minutesAgo:${Math.floor(s / 60)}:minutes: min ago`;
  return $localize`:@@live.tailStatus.hoursAgo:${Math.floor(s / 3600)}:hours: h ago`;
}

/** A positive whole number from an input, or null for empty / invalid ("off"). */
export function parseLimit(text: string): number | null {
  const t = text.trim();
  if (!/^\d+$/.test(t)) return null;
  const n = Number(t);
  return n > 0 ? n : null;
}

/** Dock view: per-file live tail state and the live tail settings (all sent with `set_live`). */
@Component({
  selector: 'imt-tail-status-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [NextSteps],
  template: `
    <form class="controls" (submit)="$event.preventDefault()" aria-label="Live tail settings" i18n-aria-label="@@live.tailStatus.settingsAria">
      <button
        type="button"
        class="btn"
        [class.primary]="live.enabled()"
        [attr.aria-pressed]="live.enabled()"
        [disabled]="!live.available()"
        (click)="live.toggle()"
      >{{ toggleLabel() }}</button>
      <label class="lbl"><ng-container i18n="@@live.tailStatus.watch">Watch</ng-container>
        <select class="select" (change)="setWatch($any($event.target).value)">
          <option value="auto" [selected]="opts().watch === 'auto'" i18n="@@live.tailStatus.watchAuto">Auto</option>
          <option value="events" [selected]="opts().watch === 'events'" i18n="@@live.tailStatus.watchEvents">File events</option>
          <option value="polling" [selected]="opts().watch === 'polling'" i18n="@@live.tailStatus.watchPolling">Polling</option>
        </select>
      </label>
      <label class="lbl"><ng-container i18n="@@live.tailStatus.pollEvery">Poll every (ms)</ng-container>
        <input class="input mono num" type="number" min="100" step="100" [value]="opts().pollIntervalMs ?? 2000" [disabled]="opts().watch === 'events'" (change)="setPoll($any($event.target).value)" />
      </label>
      <label class="lbl"><ng-container i18n="@@live.tailStatus.keepEvents">Keep at most (events)</ng-container>
        <input class="input mono num" type="number" min="1" placeholder="no limit" i18n-placeholder="@@live.tailStatus.noLimit" [value]="opts().retention?.maxRecords ?? ''" (change)="setMaxRecords($any($event.target).value)" />
      </label>
      <label class="lbl"><ng-container i18n="@@live.tailStatus.keepMinutes">Keep at most (minutes)</ng-container>
        <input class="input mono num" type="number" min="1" placeholder="no limit" i18n-placeholder="@@live.tailStatus.noLimit" [value]="maxAgeMinutes()" (change)="setMaxAge($any($event.target).value)" />
      </label>
      <label class="lbl"><ng-container i18n="@@live.tailStatus.deletedFiles">Deleted files</ng-container>
        <select class="select" (change)="setDeleted($any($event.target).value)">
          <option value="keep" [selected]="opts().deletedFiles !== 'drop'" i18n="@@live.tailStatus.deletedKeep">Keep their events</option>
          <option value="drop" [selected]="opts().deletedFiles === 'drop'" i18n="@@live.tailStatus.deletedDrop">Drop their events</option>
        </select>
      </label>
      <label class="lbl"><ng-container i18n="@@live.tailStatus.flush">Flush partial line after (ms)</ng-container>
        <input class="input mono num" type="number" min="0" step="100" [value]="opts().carryFlushMs ?? 2000" (change)="setCarry($any($event.target).value)" />
      </label>
      <label class="lbl" title="The live anomaly detectors (error spikes, crashes, slow outliers...) run at most this often. 0 runs them after every batch." i18n-title="@@live.tailStatus.detectTitle"><ng-container i18n="@@live.tailStatus.detectEvery">Run detectors every (ms)</ng-container>
        <input
          class="input mono num"
          type="number"
          min="0"
          [max]="maxDetect"
          step="100"
          data-testid="detect-every"
          [value]="opts().detectEveryMs ?? 1000"
          (change)="setDetect($any($event.target))"
        />
      </label>
    </form>
    @if (live.error(); as e) { <div class="err" role="alert">{{ e }}</div> }

    @if (status.status(); as st) {
      <div class="totals" role="status" aria-live="off">
        <span i18n="@@live.tailStatus.totalFiles"><strong>{{ st.totals.files }}</strong> files</span>
        <span i18n="@@live.tailStatus.totalTailing"><strong>{{ st.totals.tailing }}</strong> tailing</span>
        @if (st.totals.errors) { <span class="bad" i18n="@@live.tailStatus.totalErrors"><strong>{{ st.totals.errors }}</strong> with errors</span> }
        <span>{{ bytes(st.totals.bytes) }}</span>
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
                <td class="mono r">{{ bytes(f.offset) }} / {{ bytes(f.size) }}</td>
                <td class="mono r">{{ rate(f.linesPerSec) }}</td>
                <td class="nowrap">{{ ago(f.lastEvent) }}</td>
                <td [title]="hint(f)"><span class="state" [class]="f.state.kind">{{ label(f) }}</span>@if (f.state.kind === 'error') { <span class="err-msg">{{ errorOf(f) }}</span> }</td>
              </tr>
            } @empty {
              <tr><td colspan="7" class="faint" i18n="@@live.tailStatus.noFiles">No files are open. <imt-next-steps /></td></tr>
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
    .controls { flex: none; display: flex; flex-wrap: wrap; align-items: flex-end; gap: 10px; padding: 10px 12px; border-bottom: 1px solid var(--border); }
    .lbl { display: flex; flex-direction: column; gap: 3px; font-size: 11px; color: var(--muted); }
    .num { width: 110px; }
    .err { color: var(--danger-text); padding: 6px 12px; font-size: 12px; }
    .totals { flex: none; display: flex; flex-wrap: wrap; gap: 4px 16px; padding: 6px 12px; font-size: 12px; color: var(--text-2); border-bottom: 1px solid var(--border); }
    .totals .bad { color: var(--danger-text); }
    .list { flex: 1; min-height: 0; overflow: auto; }
    table { width: 100%; border-collapse: collapse; font-size: 12px; }
    th { position: sticky; top: 0; background: var(--bg-2); text-align: left; font-weight: 600; color: var(--muted); padding: 5px 8px; border-bottom: 1px solid var(--border); white-space: nowrap; }
    td { padding: 4px 8px; border-bottom: 1px solid var(--border); color: var(--text-2); max-width: 280px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .r { text-align: right; }
    .nowrap { white-space: nowrap; }
    .state { display: inline-block; padding: 0 6px; border-radius: 8px; border: 1px solid var(--border-2); font-size: 11px; }
    .state.rotated, .state.truncated { border-color: var(--warn-text); color: var(--warn-text); }
    .state.deleted { border-style: dashed; }
    .state.error { border-color: var(--danger-text); color: var(--danger-text); }
    .err-msg { margin-left: 6px; color: var(--danger-text); }
  `,
})
export class TailStatusView {
  protected readonly live = inject(LiveTail);
  protected readonly status = inject(TailStatusStore);
  private readonly cdr = inject(ChangeDetectorRef);
  protected readonly opts = computed(() => this.live.options());
  protected readonly maxAgeMinutes = computed(() => {
    const ms = this.opts().retention?.maxAgeMs;
    return ms ? Math.round(ms / 60_000) : '';
  });
  private readonly now = signal(Date.now());

  protected readonly toggleLabel = computed(() => (this.live.enabled() ? $localize`:@@live.tailStatus.liveOn:Live tail on` : $localize`:@@live.tailStatus.liveOff:Live tail off`));
  protected readonly maxDetect = MAX_DETECT_EVERY_MS;
  protected readonly count = formatCount;
  protected readonly bytes = formatBytes;
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
    return formatAgo(ts, this.now());
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

  protected setWatch(watch: WatchPreference): void {
    this.apply({ watch });
  }

  protected setPoll(text: string): void {
    const ms = parseLimit(text);
    this.apply({ pollIntervalMs: ms === null ? null : Math.max(100, ms) });
  }

  protected setMaxRecords(text: string): void {
    this.apply({ retention: { ...this.opts().retention, maxRecords: parseLimit(text) } });
  }

  protected setMaxAge(text: string): void {
    const minutes = parseLimit(text);
    this.apply({ retention: { ...this.opts().retention, maxAgeMs: minutes === null ? null : minutes * 60_000 } });
  }

  protected setDeleted(deletedFiles: DeletedFiles): void {
    this.apply({ deletedFiles });
  }

  protected setCarry(text: string): void {
    const t = text.trim();
    const ms = /^\d+$/.test(t) ? Number(t) : 2000;
    this.apply({ carryFlushMs: ms });
  }

  /** Applies a valid interval (clamped to the engine's range); an invalid entry shows the current value again. */
  protected setDetect(input: HTMLInputElement): void {
    const ms = parseDetectEvery(input.value);
    if (ms === null) {
      input.value = String(this.opts().detectEveryMs ?? 1000);
      return;
    }
    input.value = String(ms);
    this.apply({ detectEveryMs: ms });
  }

  /**
   * Applies the settings and renders them at once: the inputs' `[value]` bindings changed, and a render left for later
   * (late when the app is busy with live data) would overwrite whatever the user has typed into the field meanwhile.
   */
  private apply(patch: Partial<LiveOptions>): void {
    void this.live.setOptions({ ...this.opts(), ...patch });
    this.cdr.detectChanges();
  }
}
