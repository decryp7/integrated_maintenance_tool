import { ChangeDetectionStrategy, Component, computed, DestroyRef, effect, inject, signal, untracked } from '@angular/core';
import { DeletedFiles, LiveOptions, MAX_DETECT_EVERY_MS, TailFileStatus, WatchPreference } from '../core/api.types';
import { formatCount } from '../core/format';
import { LiveTail } from '../workspace/live-tail';
import { parseDetectEvery } from './live-ui-state';
import { stateHint, stateLabel, TailStatusStore } from './tail-status.store';

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
  return `${v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
}

/** "just now", "12 s ago", "3 min ago", "2 h ago" */
export function formatAgo(ts: number | null, now: number): string {
  if (ts === null) return '—';
  const s = Math.max(0, Math.round((now - ts) / 1000));
  if (s < 2) return 'just now';
  if (s < 60) return `${s} s ago`;
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  return `${Math.floor(s / 3600)} h ago`;
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
  template: `
    <form class="controls" (submit)="$event.preventDefault()" aria-label="Live tail settings">
      <button
        type="button"
        class="btn"
        [class.primary]="live.enabled()"
        [attr.aria-pressed]="live.enabled()"
        [disabled]="!live.available()"
        (click)="live.toggle()"
      >Live tail {{ live.enabled() ? 'on' : 'off' }}</button>
      <label class="lbl">Watch
        <select class="select" (change)="setWatch($any($event.target).value)">
          <option value="auto" [selected]="opts().watch === 'auto'">Auto</option>
          <option value="events" [selected]="opts().watch === 'events'">File events</option>
          <option value="polling" [selected]="opts().watch === 'polling'">Polling</option>
        </select>
      </label>
      <label class="lbl">Poll every (ms)
        <input class="input mono num" type="number" min="100" step="100" [value]="opts().pollIntervalMs ?? 2000" [disabled]="opts().watch === 'events'" (change)="setPoll($any($event.target).value)" />
      </label>
      <label class="lbl">Keep at most (events)
        <input class="input mono num" type="number" min="1" placeholder="no limit" [value]="opts().retention?.maxRecords ?? ''" (change)="setMaxRecords($any($event.target).value)" />
      </label>
      <label class="lbl">Keep at most (minutes)
        <input class="input mono num" type="number" min="1" placeholder="no limit" [value]="maxAgeMinutes()" (change)="setMaxAge($any($event.target).value)" />
      </label>
      <label class="lbl">Deleted files
        <select class="select" (change)="setDeleted($any($event.target).value)">
          <option value="keep" [selected]="opts().deletedFiles !== 'drop'">Keep their events</option>
          <option value="drop" [selected]="opts().deletedFiles === 'drop'">Drop their events</option>
        </select>
      </label>
      <label class="lbl">Flush partial line after (ms)
        <input class="input mono num" type="number" min="0" step="100" [value]="opts().carryFlushMs ?? 2000" (change)="setCarry($any($event.target).value)" />
      </label>
      <label class="lbl" title="The live anomaly detectors (error spikes, crashes, slow outliers...) run at most this often. 0 runs them after every batch.">Run detectors every (ms)
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
        <span><strong>{{ st.totals.files }}</strong> files</span>
        <span><strong>{{ st.totals.tailing }}</strong> tailing</span>
        @if (st.totals.errors) { <span class="bad"><strong>{{ st.totals.errors }}</strong> with errors</span> }
        <span>{{ bytes(st.totals.bytes) }}</span>
        <span><strong>{{ rate(st.totals.linesPerSec) }}</strong> lines/s</span>
        <span>{{ count(st.totals.records) }} events</span>
        <span [title]="'Events removed by the retention limit since the workspace was opened'">{{ count(st.totals.evicted) }} evicted</span>
      </div>
      <div class="list" tabindex="0" role="region" aria-label="Tailed files">
        <table>
          <thead>
            <tr>
              <th scope="col">File</th><th scope="col">Group</th><th scope="col">Mode</th><th scope="col" class="r">Offset / size</th>
              <th scope="col" class="r">Lines/s</th><th scope="col">Last change</th><th scope="col">State</th>
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
              <tr><td colspan="7" class="faint">No files are open.</td></tr>
            }
          </tbody>
        </table>
      </div>
    } @else if (status.error(); as e) {
      <div class="empty-state"><div class="err">The tail status could not be loaded: {{ e }}</div><button type="button" class="btn" (click)="status.refresh()">Retry</button></div>
    } @else {
      <div class="empty-state faint" role="status">Loading…</div>
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
  protected readonly opts = computed(() => this.live.options());
  protected readonly maxAgeMinutes = computed(() => {
    const ms = this.opts().retention?.maxAgeMs;
    return ms ? Math.round(ms / 60_000) : '';
  });
  private readonly now = signal(Date.now());

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
    return v >= 100 ? Math.round(v).toString() : v.toFixed(1);
  }

  protected modeLabel(f: TailFileStatus): string {
    return f.state.kind === 'whole' ? 'Whole file' : f.mode === 'events' ? 'Events' : f.mode === 'polling' ? 'Polling' : 'Off';
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

  private apply(patch: Partial<LiveOptions>): void {
    void this.live.setOptions({ ...this.opts(), ...patch });
  }
}
