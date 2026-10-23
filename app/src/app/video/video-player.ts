import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, input, output, viewChild } from '@angular/core';
import { DomSanitizer } from '@angular/platform-browser';
import { VideoView } from '../core/generated/video';
import { VIDEO_STYLES } from './video-styles';

/** The only scheme and host the shell serves videos from (ADR 0033 section 7). */
export const MEDIA_PREFIX = 'media://localhost/';

/**
 * Plays the video the shell serves under `media://localhost/<key>` in the browser's own player. A video that is not
 * where it was (another computer, a moved file) is explained, with "Locate the video…": frames and notes still work.
 */
@Component({
  selector: 'debrief-video-player',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (video().media === 'missing') {
      <div class="card missing" role="status" data-testid="video-missing">
        <p>
          <strong i18n="@@video.player.missing.title">The video file is not where it was.</strong>
          <ng-container i18n="@@video.player.missing.text">It may have been moved or analysed on another computer. The frames, notes and descriptions still work.</ng-container>
        </p>
        @if (video().record.path; as path) {
          <p class="muted mono" data-testid="video-missing-path">{{ path }}</p>
        }
        <button class="btn" type="button" data-testid="video-locate" (click)="locate.emit()" i18n="@@video.player.locate">Locate the video…</button>
      </div>
    } @else if (url(); as src) {
      <video #player controls preload="metadata" [src]="src" [attr.aria-label]="label()" (timeupdate)="onTime()" (loadedmetadata)="onTime()"></video>
    } @else {
      <div class="card missing" role="status" data-testid="video-unplayable">
        <p class="muted" i18n="@@video.player.unplayable">This video cannot be played here. The frames below are still available.</p>
      </div>
    }
  `,
  styles: [VIDEO_STYLES, `video { display: block; width: 100%; max-height: 42vh; background: var(--bg-0); border-radius: 4px; } .missing p { margin: 0 0 8px; }`],
})
export class VideoPlayer {
  private readonly sanitizer = inject(DomSanitizer);
  readonly video = input.required<VideoView>();
  /** Move the player here (ms in the video) when it changes, e.g. when a frame is chosen. */
  readonly seekMs = input<number | null>(null);
  readonly locate = output<void>();
  /** The player's position (ms), as it plays or is dragged. */
  readonly time = output<number>();

  private readonly player = viewChild<ElementRef<HTMLVideoElement>>('player');

  /** Only the shell's own `media:` address is passed to the player. */
  protected readonly url = computed(() => {
    const u = this.video().mediaUrl;
    return u && u.startsWith(MEDIA_PREFIX) ? this.sanitizer.bypassSecurityTrustUrl(u) : null;
  });

  protected readonly label = computed(() => $localize`:@@video.player.label:Video ${this.video().record.name}:name:`);

  constructor() {
    effect(() => {
      const ms = this.seekMs();
      const el = this.player()?.nativeElement;
      if (ms !== null && el) el.currentTime = ms / 1000;
    });
  }

  protected onTime(): void {
    const el = this.player()?.nativeElement;
    if (el) this.time.emit(Math.round(el.currentTime * 1000));
  }
}
