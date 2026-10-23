import { ChangeDetectionStrategy, Component, DestroyRef, effect, ElementRef, inject, input, signal, untracked } from '@angular/core';
import { VideoStore } from './video.store';

/**
 * The picture of one frame, asked from the engine when it scrolls into view (the strip can hold 120 frames) or at once
 * when `eager`. The engine returns a data URL; it is shown in an `<img>`, never as markup.
 */
@Component({
  selector: 'debrief-frame-image',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (src(); as s) {
      <img [src]="s" [alt]="alt()" />
    } @else {
      <!-- With no alt the picture is decorative (the strip's option names the frame): nothing for a screen reader. -->
      <span class="ph" [attr.role]="alt() ? 'img' : null" [attr.aria-label]="alt() || null" [attr.aria-hidden]="alt() ? null : 'true'">{{ failed() ? '!' : '' }}</span>
    }
  `,
  styles: `
    :host { display: block; background: var(--bg-3); }
    img, .ph { display: block; width: 100%; height: 100%; object-fit: contain; }
    .ph { display: flex; align-items: center; justify-content: center; min-height: 40px; color: var(--muted); }
  `,
})
export class FrameImage {
  private readonly store = inject(VideoStore);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  private readonly destroyRef = inject(DestroyRef);

  readonly videoKey = input.required<string>();
  readonly index = input.required<number>();
  /** The frame's image file, so a different picture under the same index is loaded again. */
  readonly image = input('');
  readonly alt = input('');
  readonly eager = input(false);

  protected readonly src = signal<string | null>(null);
  protected readonly failed = signal(false);
  private visible = signal(false);

  constructor() {
    if (typeof IntersectionObserver === 'undefined') {
      this.visible.set(true);
    } else {
      const observer = new IntersectionObserver((entries) => {
        if (entries.some((e) => e.isIntersecting)) this.visible.set(true);
      });
      observer.observe(this.host.nativeElement);
      this.destroyRef.onDestroy(() => observer.disconnect());
    }
    // effect-state: IPC frame fetch once visible; the image lands in state when the engine answers
    effect(() => {
      const key = this.videoKey();
      const index = this.index();
      this.image();
      if (!this.eager() && !this.visible()) return;
      untracked(() => {
        this.failed.set(false);
        this.store.frameImage(key, index).then(
          (url) => {
            if (key === this.videoKey() && index === this.index()) this.src.set(url);
          },
          () => this.failed.set(true),
        );
      });
    });
  }
}
