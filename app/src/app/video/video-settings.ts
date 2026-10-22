import { ChangeDetectionStrategy, Component, computed, inject, OnInit } from '@angular/core';
import { formatBytes } from '../shared/format/units';
import { RuntimeMode } from '../core/generated/video';
import { VideoError } from './video-error';
import { VIDEO_STYLES } from './video-styles';
import { VideoStore } from './video.store';

/** What each mode of the runtime means, in a sentence. */
export function modeText(mode: RuntimeMode): string {
  switch (mode) {
    case 'none':
      return $localize`:@@video.settings.mode.none:No FFmpeg is set up yet.`;
    case 'managed':
      return $localize`:@@video.settings.mode.managed:IMT's downloaded FFmpeg is used.`;
    case 'custom':
      return $localize`:@@video.settings.mode.custom:The ffmpeg you chose is used.`;
  }
}

/**
 * Settings ▸ Video: the FFmpeg runtime (ADR 0033). IMT does not ship FFmpeg: where a pinned LGPL build exists it is
 * downloaded once, checked and self-tested (a job; progress is in Jobs), with Roll back to the build before. You can
 * instead use an ffmpeg of your own (e.g. Homebrew's); the engine tests it before it is used.
 */
@Component({
  selector: 'imt-video-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [VideoError],
  template: `
    <h3 class="title" i18n="Settings section@@video.settings.title">Video</h3>
    <p class="desc" i18n="@@video.settings.desc">IMT reads videos with FFmpeg: it picks the frames where the picture changes and lines them up with the logs. FFmpeg is not part of IMT.</p>
    <h4 i18n="@@video.settings.runtime">FFmpeg runtime</h4>
    @if (store.errors().runtime; as e) {
      <imt-video-error [failure]="e" (retry)="store.retry('runtime')" (dismiss)="store.dismiss('runtime')" />
    }
    @if (store.runtime(); as r) {
      @if (r.status.problem; as problem) {
        <p class="problem" role="alert" data-testid="video-settings-problem">{{ problem }}</p>
      }
      <p data-testid="video-settings-mode">{{ mode() }}</p>
      @if (r.status.active; as a) {
        <p data-testid="video-settings-active" i18n="@@video.settings.active">Downloaded build: <strong>{{ a.tag }}</strong>@if (a.version) { <span> (FFmpeg {{ a.version }})</span> }, {{ bytes(a.bytes) }}</p>
      }
      @if (r.status.previous; as p) {
        <p class="muted" data-testid="video-settings-previous" i18n="@@video.settings.previous">Build before: {{ p.tag }}@if (p.version) { <span> (FFmpeg {{ p.version }})</span> }</p>
      }
      @if (r.canDownload) {
        <p class="muted" data-testid="video-settings-pinned">
          @if (r.status.pinnedRelease; as rel) {
            <ng-container i18n="@@video.settings.pinned">IMT offers {{ rel }} ({{ pinnedSize() }}), checked against its checksum and tested before use.</ng-container>
          }
          @if (r.status.updateAvailable && r.status.active) {
            <strong data-testid="video-settings-update" i18n="@@video.settings.updateAvailable"> An update is available.</strong>
          }
        </p>
        <div class="row">
          <button class="btn primary" type="button" data-testid="video-install" [disabled]="busy() || (!!r.status.active && !r.status.updateAvailable && !r.status.problem)" (click)="store.installRuntime()">{{ installLabel() }}</button>
          <button class="btn" type="button" data-testid="video-rollback" [disabled]="busy() || !r.status.previous" (click)="store.rollbackRuntime()" i18n="@@video.settings.rollback">Roll back</button>
          <button class="btn" type="button" data-testid="video-delete-old" [disabled]="busy() || !r.status.previous" (click)="deleteOld()" i18n="@@video.settings.deleteOld">Delete the old build</button>
        </div>
        @if (store.runtimeBusy() === 'install') {
          <p class="muted" role="status" data-testid="video-installing" i18n="@@video.settings.installing">Downloading and testing FFmpeg. You can keep working; progress and Cancel are in Jobs.</p>
        }
      } @else if (isMac()) {
        <p class="note" role="note" data-testid="video-settings-mac" i18n="@@video.settings.mac">There is no FFmpeg build for IMT to download on macOS yet. Install one with Homebrew (<span class="mono">brew install ffmpeg</span>) and use it here.</p>
      } @else {
        <p class="note" role="note" data-testid="video-settings-nodownload" i18n="@@video.settings.noDownload">IMT has no FFmpeg build to download for this computer. Install ffmpeg yourself and use it here.</p>
      }

      <h4 i18n="@@video.settings.custom">Your own ffmpeg</h4>
      @if (r.status.customPath; as path) {
        <p data-testid="video-settings-custom"><span class="mono">{{ path }}</span>@if (r.status.customVersion; as v) { <span> (FFmpeg {{ v }})</span> }</p>
      } @else {
        <p class="muted" i18n="@@video.settings.noCustom">None chosen.</p>
      }
      <div class="row">
        <button class="btn" type="button" data-testid="video-choose" [disabled]="busy()" (click)="store.chooseRuntime()" i18n="@@video.settings.choose">Choose ffmpeg…</button>
        <button class="btn" type="button" data-testid="video-stop-custom" [disabled]="busy() || !r.status.customPath" (click)="store.clearCustomRuntime()" i18n="@@video.settings.stop">Stop using it</button>
      </div>
      @if (r.detected.length) {
        <p class="muted" i18n="@@video.settings.found">Found on this computer:</p>
        <ul class="found" aria-label="ffmpeg programs found" i18n-aria-label="@@video.settings.foundLabel">
          @for (d of r.detected; track d.path) {
            <li data-testid="video-detected">
              <span class="mono grow">{{ d.path }}</span>
              <span class="muted">{{ d.origin }}</span>
              @if (d.path === r.status.customPath) {
                <span class="muted" i18n="@@video.settings.inUse">In use</span>
              } @else {
                <button class="btn" type="button" [disabled]="busy()" [attr.aria-label]="useLabel(d.path)" (click)="store.useCustomRuntime(d.path)" i18n="@@video.settings.use">Use this one</button>
              }
            </li>
          }
        </ul>
      }
    } @else if (!store.errors().runtime) {
      <p class="muted" role="status" i18n="@@video.settings.loading">Checking FFmpeg…</p>
    }
    @if (store.runtimeBusy() === 'custom') {
      <p class="muted" role="status" i18n="@@video.settings.testing">Testing this ffmpeg…</p>
    }
  `,
  styles: [
    VIDEO_STYLES,
    `
      :host { display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-md); }
      .title { margin: 0; font-size: var(--fs-lg); font-weight: 600; letter-spacing: 0; text-transform: none; color: var(--text); }
      h4 { margin: 12px 0 2px; font-size: var(--fs-md); color: var(--text); }
      .desc { margin: 0; color: var(--muted); line-height: 1.45; }
      .problem { padding: 8px 10px; border: 1px solid var(--danger-text); border-radius: 6px; color: var(--danger-text); }
      .note { color: var(--muted); }
      .found { list-style: none; margin: 0; padding: 0; }
      .found li { display: flex; align-items: center; gap: 10px; padding: 4px 0; border-bottom: 1px solid var(--border); }
    `,
  ],
})
export class VideoSettings implements OnInit {
  protected readonly store = inject(VideoStore);
  protected readonly bytes = formatBytes;

  protected readonly busy = computed(() => this.store.runtimeBusy() !== null);
  protected readonly mode = computed(() => modeText(this.store.runtime()?.status.mode ?? 'none'));
  protected readonly isMac = computed(() => this.store.runtime()?.platform.startsWith('macos') ?? false);
  protected readonly pinnedSize = computed(() => formatBytes(this.store.runtime()?.status.pinnedBytes ?? 0));
  protected readonly installLabel = computed(() => {
    const s = this.store.runtime()?.status;
    const size = this.pinnedSize();
    if (s?.problem) return $localize`:@@video.settings.installAgain:Install again (${size}:size:)`;
    if (s?.active && s.updateAvailable) return $localize`:@@video.settings.update:Update (${size}:size:)`;
    return $localize`:@@video.settings.install:Download FFmpeg (${size}:size:)`;
  });

  ngOnInit(): void {
    void this.store.loadRuntime();
  }

  protected useLabel(path: string): string {
    return $localize`:@@video.settings.useLabel:Use ${path}:path:`;
  }

  protected deleteOld(): void {
    const tag = this.store.runtime()?.status.previous?.tag;
    if (tag) void this.store.deleteRuntime(tag);
  }
}
