import { ChangeDetectionStrategy, ChangeDetectorRef, Component, computed, inject } from '@angular/core';
import { DeletedFiles, LiveOptions, MAX_DETECT_EVERY_MS, WatchPreference } from '../core/api.types';
import { LiveTail } from '../workspace/live-tail';
import { parseDetectEvery } from './live-ui-state';

/** A positive whole number from an input, or null for empty / invalid ("off"). */
export function parseLimit(text: string): number | null {
  const t = text.trim();
  if (!/^\d+$/.test(t)) return null;
  const n = Number(t);
  return n > 0 ? n : null;
}

/**
 * Settings ▸ Live tail, under the engine's own field (the re-import delay): how files are watched,
 * what happens to deleted files and how often the detectors run. Every control is sent with `set_live`, which the
 * engine validates (clamps) again; units are suffixes, not part of the label.
 */
@Component({
  selector: 'imt-live-tail-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <form class="form" (submit)="$event.preventDefault()" aria-label="Live tail options" i18n-aria-label="@@live.settings.aria">
      <label class="lbl"><span i18n="@@live.tailStatus.watch">Watch</span>
        <select class="select" (change)="setWatch($any($event.target).value)">
          <option value="auto" [selected]="opts().watch === 'auto'" i18n="@@live.tailStatus.watchAuto">Auto</option>
          <option value="events" [selected]="opts().watch === 'events'" i18n="@@live.tailStatus.watchEvents">File events</option>
          <option value="polling" [selected]="opts().watch === 'polling'" i18n="@@live.tailStatus.watchPolling">Polling</option>
        </select>
      </label>
      <label class="lbl"><span i18n="@@live.settings.pollEvery">Poll every</span>
        <span class="with-unit"><input class="input mono num" type="number" min="100" step="100" [value]="opts().pollIntervalMs ?? 2000" [disabled]="opts().watch === 'events'" (change)="setPoll($any($event.target).value)" /><span class="unit" aria-hidden="true">ms</span></span>
      </label>
      <label class="lbl"><span i18n="@@live.tailStatus.deletedFiles">Deleted files</span>
        <select class="select" (change)="setDeleted($any($event.target).value)">
          <option value="keep" [selected]="opts().deletedFiles !== 'drop'" i18n="@@live.tailStatus.deletedKeep">Keep their events</option>
          <option value="drop" [selected]="opts().deletedFiles === 'drop'" i18n="@@live.tailStatus.deletedDrop">Drop their events</option>
        </select>
      </label>
      <label class="lbl"><span i18n="@@live.settings.flush">Flush a partial line after</span>
        <span class="with-unit"><input class="input mono num" type="number" min="0" step="100" [value]="opts().carryFlushMs ?? 500" (change)="setCarry($any($event.target).value)" /><span class="unit" aria-hidden="true">ms</span></span>
      </label>
      <label class="lbl" title="The live anomaly detectors (error spikes, crashes, slow outliers...) run at most this often. 0 runs them after every batch." i18n-title="@@live.tailStatus.detectTitle"><span i18n="@@live.settings.detectEvery">Run detectors every</span>
        <span class="with-unit"><input class="input mono num" type="number" min="0" [max]="maxDetect" step="100" data-testid="detect-every" [value]="opts().detectEveryMs ?? 1000" (change)="setDetect($any($event.target))" /><span class="unit" aria-hidden="true">ms</span></span>
      </label>
    </form>
    @if (live.error(); as e) { <p class="err" role="alert">{{ e }}</p> }
  `,
  styles: `
    :host { display: block; margin-top: 12px; }
    .form { display: grid; grid-template-columns: repeat(auto-fill, minmax(230px, 1fr)); gap: 12px 16px; }
    .lbl { display: flex; flex-direction: column; gap: 4px; color: var(--text-2); }
    .with-unit { display: flex; align-items: center; gap: 6px; }
    .unit { color: var(--muted); font-size: var(--fs-sm); }
    .num { width: 120px; }
    .err { color: var(--danger-text); }
  `,
})
export class LiveTailSettings {
  protected readonly live = inject(LiveTail);
  private readonly cdr = inject(ChangeDetectorRef);
  protected readonly opts = computed(() => this.live.options());
  protected readonly maxDetect = MAX_DETECT_EVERY_MS;

  protected setWatch(watch: WatchPreference): void {
    this.apply({ watch });
  }

  protected setPoll(text: string): void {
    const ms = parseLimit(text);
    this.apply({ pollIntervalMs: ms === null ? null : Math.max(100, ms) });
  }

  protected setDeleted(deletedFiles: DeletedFiles): void {
    this.apply({ deletedFiles });
  }

  protected setCarry(text: string): void {
    const t = text.trim();
    const ms = /^\d+$/.test(t) ? Number(t) : 500;
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
