import { ChangeDetectionStrategy, Component, computed, inject, linkedSignal, signal } from '@angular/core';
import { formatDateTime } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { SettingsStore } from '../settings/settings.store';
import { VideoError } from './video-error';
import { alignedByLabel, confidenceLabel, evidenceText, fromInputValue, parseOffsetSeconds, toInputValue, userAlignmentText } from './video-format';
import { VIDEO_STYLES } from './video-styles';
import { VideoStore } from './video.store';

/**
 * Where the video's time 0 is on the logs' clock and how that was decided, with the evidence in words. The engine
 * decides (a start you set, a clock on the screen, the file name, the container, the file time); here the user can set
 * the start, shift it, go back to Auto, or have the model read the clock in the chosen frame.
 */
@Component({
  selector: 'debrief-alignment-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [VideoError],
  template: `
    <h3 i18n="@@video.align.title">Alignment with the logs</h3>
    @if (store.errors().align; as e) {
      <debrief-video-error [failure]="e" (retry)="store.retry('align')" (dismiss)="store.dismiss('align')" />
    }
    @if (store.active(); as v) {
      <p data-testid="video-align-start"><ng-container i18n="@@video.align.start">The video starts at</ng-container> <strong class="mono">{{ start() }}</strong></p>
      <p data-testid="video-align-by">
        <ng-container i18n="@@video.align.decidedBy">Decided by</ng-container> {{ by() }} ·
        <ng-container i18n="@@video.align.confidence">confidence</ng-container> <strong data-testid="video-align-confidence">{{ confidence() }}</strong>
      </p>
      <p class="muted" data-testid="video-align-evidence">{{ evidence() }}</p>
      <p class="muted" data-testid="video-align-user"><ng-container i18n="@@video.align.yours">Your setting</ng-container>: {{ yours() }}</p>
      <div class="row">
        <label class="field">
          <span i18n="@@video.align.setStart">Start at</span>
          <input type="datetime-local" step="1" data-testid="video-align-input" [value]="startInput()" (input)="startInput.set(valueOf($event))" />
        </label>
        <button class="btn" type="button" data-testid="video-align-set" [disabled]="store.busy() !== null || typedStart() === null" (click)="setStart()" i18n="@@video.align.setStartButton">Set start</button>
      </div>
      <div class="row">
        <label class="field">
          <span i18n="@@video.align.shift">Move by (seconds)</span>
          <input type="text" inputmode="decimal" data-testid="video-align-offset" [value]="offsetText()" (input)="offsetText.set(valueOf($event))" placeholder="-30 or 1.5" />
        </label>
        <button class="btn" type="button" data-testid="video-align-shift" [disabled]="store.busy() !== null || offsetMs() === null" (click)="shift()" i18n="@@video.align.shiftButton">Move</button>
      </div>
      <div class="row">
        <button class="btn" type="button" data-testid="video-align-auto" [disabled]="store.busy() !== null || v.record.userAlignment.kind === 'auto'" (click)="store.setAlignment({ kind: 'auto' })" i18n="@@video.align.auto">Reset to Auto</button>
        <button class="btn" type="button" data-testid="video-align-clock" [disabled]="store.busy() !== null || store.selectedIndex() === null" (click)="readClock()" i18n="@@video.align.readClock">Read the clock from this frame</button>
      </div>
      @if (store.busy() === 'clock') {
        <p class="muted" role="status" i18n="@@video.align.reading">The model is reading the clock. Progress and Cancel are in Jobs.</p>
      }
    }
  `,
  styles: [VIDEO_STYLES],
})
export class AlignmentPanel {
  protected readonly store = inject(VideoStore);
  private readonly workspace = inject(WorkspaceStore);
  private readonly settings = inject(SettingsStore);

  protected readonly offsetText = signal('');

  private readonly tz = this.workspace.timezone;
  protected readonly alignment = computed(() => this.store.active()?.record.alignment ?? null);
  private readonly currentStartMs = computed(() => this.alignment()?.startMs ?? null);
  /** The start field shows the current start whenever it (or the time zone) changes; typing replaces it meanwhile. */
  protected readonly startInput = linkedSignal<string, string>({
    source: computed(() => {
      const ms = this.currentStartMs();
      return ms === null ? '' : toInputValue(ms, this.tz());
    }),
    computation: (shown) => shown,
  });
  protected readonly start = computed(() => {
    const a = this.alignment();
    return a ? formatDateTime(a.startMs, this.tz(), true, this.settings.dateFormat()) : '';
  });
  protected readonly by = computed(() => {
    const a = this.alignment();
    return a ? alignedByLabel(a.by) : '';
  });
  protected readonly confidence = computed(() => {
    const a = this.alignment();
    return a ? confidenceLabel(a.confidence) : '';
  });
  protected readonly evidence = computed(() => {
    const a = this.alignment();
    return a ? evidenceText(a.evidence, this.tz(), this.settings.dateFormat()) : '';
  });
  protected readonly yours = computed(() => {
    const u = this.store.active()?.record.userAlignment;
    return u ? userAlignmentText(u, this.tz(), this.settings.dateFormat()) : '';
  });
  protected readonly typedStart = computed(() => fromInputValue(this.startInput(), this.tz()));
  protected readonly offsetMs = computed(() => parseOffsetSeconds(this.offsetText()));

  protected valueOf(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }

  protected async setStart(): Promise<void> {
    const startMs = this.typedStart();
    if (startMs !== null) await this.store.setAlignment({ kind: 'start', startMs });
  }

  protected async shift(): Promise<void> {
    const offsetMs = this.offsetMs();
    if (offsetMs === null) return;
    if (await this.store.setAlignment({ kind: 'offset', offsetMs })) this.offsetText.set('');
  }

  protected async readClock(): Promise<void> {
    const i = this.store.selectedIndex();
    if (i !== null) await this.store.readClock(i);
  }
}
