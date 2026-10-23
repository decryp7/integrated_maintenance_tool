import { ChangeDetectionStrategy, Component, computed, inject, OnInit, signal } from '@angular/core';
import { SettingsDialogService } from '../settings/settings-dialog';
import { AlignmentPanel } from './alignment-panel';
import { FrameDetail } from './frame-detail';
import { FrameStrip } from './frame-strip';
import { VideoError } from './video-error';
import { VideoPlayer } from './video-player';
import { VideoStorage } from './video-storage';
import { VIDEO_SETTINGS_ID } from './video-settings.types';
import { formatVideoTime } from './video-format';
import { VIDEO_STYLES } from './video-styles';
import { VideoStore } from './video.store';

/**
 * A video next to the logs (ADR 0033): the player, the frames the engine chose, the selected frame with its note and
 * what the AI can do with it, where the video sits on the logs' clock and what it costs the project. Everything is
 * computed by the engine; this presents it. Each part shows its own failure with Retry.
 */
@Component({
  selector: 'debrief-video-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AlignmentPanel, FrameDetail, FrameStrip, VideoError, VideoPlayer, VideoStorage],
  template: `
    <div class="bar">
      @if (store.videos().length > 1) {
        <label class="field">
          <span class="sr" i18n="@@video.view.pickText">Video</span>
          <select data-testid="video-pick" aria-label="Video" i18n-aria-label="@@video.view.pickLabel" [value]="active()?.record.key" (change)="pick($event)">
            @for (v of store.videos(); track v.record.key) {
              <option [value]="v.record.key">{{ v.record.name }}</option>
            }
          </select>
        </label>
      } @else if (active(); as v) {
        <strong class="name" data-testid="video-name">{{ v.record.name }}</strong>
      }
      @if (active(); as v) {
        <span class="muted" data-testid="video-facts">{{ v.record.width }}×{{ v.record.height }} · {{ v.record.codec }} · {{ duration() }}</span>
      }
      <span class="spacer"></span>
      <button class="btn" type="button" data-testid="video-open" [disabled]="store.opening()" (click)="store.pickAndOpen()" i18n="@@video.view.open">Open video…</button>
    </div>

    @if (store.errors().list; as e) {
      <debrief-video-error [failure]="e" (retry)="store.retry('list')" (dismiss)="store.dismiss('list')" />
    }
    @if (store.errors().open; as e) {
      <debrief-video-error [failure]="e" (retry)="store.retry('open')" (dismiss)="store.dismiss('open')" />
    }
    @if (store.errors().runtime; as e) {
      <debrief-video-error [failure]="e" (retry)="store.retry('runtime')" (dismiss)="store.dismiss('runtime')" />
    }
    @if (store.runtimeMissing()) {
      <div class="notice" role="alert" data-testid="video-runtime-problem">
        <span>
          @if (store.runtime()?.status?.problem; as problem) {
            {{ problem }}
          } @else {
            <ng-container i18n="@@video.view.noRuntime">FFmpeg is not set up, so videos cannot be analysed yet.</ng-container>
          }
        </span>
        <button class="btn" type="button" data-testid="video-runtime-setup" (click)="openSettings()" i18n="@@video.view.setup">Set up FFmpeg…</button>
      </div>
    }
    @if (store.opening()) {
      <p class="notice" role="status" data-testid="video-opening" i18n="@@video.view.opening">Analysing the video. It can take a few minutes; progress and Cancel are in Jobs.</p>
    }

    @if (active(); as v) {
      <div class="body">
        <section class="main" aria-label="Video" i18n-aria-label="@@video.view.playerRegion">
          <debrief-video-player [video]="v" [seekMs]="seek()" (time)="playerMs.set($event)" (locate)="store.locate(v.record.key)" />
          <debrief-frame-strip [playerMs]="playerMs()" (chosen)="seekTo($event)" />
          <debrief-frame-detail />
        </section>
        <aside class="side" aria-label="Alignment and storage" i18n-aria-label="@@video.view.sideRegion">
          <debrief-alignment-panel class="card" />
          <debrief-video-storage class="card" />
        </aside>
      </div>
    } @else if (!store.opening()) {
      <div class="empty" role="status" data-testid="video-empty">
        <p i18n="@@video.view.none">Open a video to see the moments that matter on the logs' clock.</p>
        <button class="btn primary" type="button" (click)="store.pickAndOpen()" i18n="@@video.view.openFirst">Open a video…</button>
        <p class="hint" i18n="@@video.view.ffmpegHint">Debrief uses FFmpeg to read videos. Set it up once in Settings ▸ Video.</p>
        <button class="link" type="button" data-testid="video-empty-settings" (click)="openSettings()" i18n="@@video.view.settings">Open Settings ▸ Video</button>
      </div>
    }
  `,
  styles: [
    VIDEO_STYLES,
    `
      :host { flex: 1; display: flex; flex-direction: column; min-height: 0; background: var(--bg-1); }
      .bar { flex: none; display: flex; align-items: center; flex-wrap: wrap; gap: 8px 10px; padding: 6px 12px; border-bottom: 1px solid var(--border); }
      .name { color: var(--text); }
      .spacer { flex: 1; }
      .notice { flex: none; display: flex; align-items: center; gap: 10px; margin: 0; padding: 6px 12px; border-bottom: 1px solid var(--border); color: var(--text-2); }
      .notice[role='alert'] { color: var(--danger-text); }
      .body { flex: 1; min-height: 0; overflow: auto; display: grid; grid-template-columns: minmax(0, 2fr) minmax(240px, 1fr); gap: 12px; padding: 10px 12px; align-items: start; }
      .main { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
      .side { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
      .side > * { display: block; }
      .empty { margin: auto; display: flex; flex-direction: column; align-items: center; gap: 8px; color: var(--muted); text-align: center; padding: 16px; }
      .hint { max-width: 420px; }
      @media (max-width: 760px) { .body { grid-template-columns: 1fr; } }
    `,
  ],
})
export class VideoView implements OnInit {
  protected readonly store = inject(VideoStore);
  private readonly settings = inject(SettingsDialogService);

  protected readonly active = this.store.active;
  protected readonly playerMs = signal(0);
  /** Where the player is moved to when a frame is chosen. */
  protected readonly seek = signal<number | null>(null);
  protected readonly duration = computed(() => formatVideoTime(this.active()?.record.durationMs ?? 0));

  ngOnInit(): void {
    if (!this.store.runtime()) void this.store.loadRuntime();
  }

  protected pick(e: Event): void {
    this.store.setActive((e.target as HTMLSelectElement).value);
  }

  protected seekTo(index: number): void {
    const frame = this.active()?.record.frames[index];
    if (frame) this.seek.set(frame.videoMs);
  }

  protected openSettings(): void {
    this.settings.open(VIDEO_SETTINGS_ID);
  }
}
