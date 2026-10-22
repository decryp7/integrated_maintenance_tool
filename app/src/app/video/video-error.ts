import { ChangeDetectionStrategy, Component, inject, Injector, input, output } from '@angular/core';
import { VideoFailure } from './video-format';
import { VIDEO_STYLES } from './video-styles';

/** The id of Settings ▸ AI (Rust `settings` section). */
const AI_SETTINGS = 'ai';

/**
 * A failure of one part of the Video view, inline: what did not work, the engine's next step, Retry when trying again
 * may help and, when the AI is not ready for it, the way to Settings ▸ AI.
 */
@Component({
  selector: 'imt-video-error',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="err" role="alert">
      <span class="msg">{{ failure().message }}</span>
      @if (failure().hint; as hint) {
        <span class="hint">{{ hint }}</span>
      }
      @if (failure().aiSetup) {
        <button class="btn" type="button" data-testid="video-error-ai" (click)="openAiSettings()" i18n="@@video.error.aiSettings">Open Settings ▸ AI</button>
      }
      @if (failure().retryable) {
        <button class="btn" type="button" data-testid="video-error-retry" (click)="retry.emit()" i18n="@@video.error.retry">Retry</button>
      }
      <button class="btn ghost" type="button" (click)="dismiss.emit()" i18n="@@video.error.dismiss">Dismiss</button>
    </div>
  `,
  styles: [VIDEO_STYLES, `.err { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 10px; padding: 6px 10px; border: 1px solid var(--danger-text); border-radius: 6px; color: var(--danger-text); } .hint { color: var(--text-2); }`],
})
export class VideoError {
  private readonly injector = inject(Injector);
  readonly failure = input.required<VideoFailure>();
  readonly retry = output<void>();
  readonly dismiss = output<void>();

  /** Loaded on demand: the Settings screen is large and this component is also part of it. */
  protected async openAiSettings(): Promise<void> {
    const { SettingsDialogService } = await import('../settings/settings-dialog');
    this.injector.get(SettingsDialogService).open(AI_SETTINGS);
  }
}
