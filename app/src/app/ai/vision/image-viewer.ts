import { ChangeDetectionStrategy, Component, computed, input, signal } from '@angular/core';
import { AiImage } from '../ai.types';
import { capturedLabel, sizeLabel } from './related-format';

/**
 * Shows one image: its clean copy (the `data:` URL Rust re-encoded, never the user's file), fit to the panel or at
 * 100 %, with its sizes and capture time, and the caller's actions in the `[actions]` slot. Presentational and
 * generic: notes and annotation (ADR 0021) reuse it.
 */
@Component({
  selector: 'imt-image-viewer',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="bar">
      <span class="name ellipsis" [title]="image().path ?? image().name">{{ image().name }}</span>
      <button type="button" class="btn ghost small" [attr.aria-pressed]="actual()" data-testid="image-zoom" (click)="actual.set(!actual())">
        @if (actual()) {
          <ng-container i18n="@@ai.vision.viewer.fit">Fit</ng-container>
        } @else {
          <ng-container i18n="@@ai.vision.viewer.actual">100 %</ng-container>
        }
      </button>
      <ng-content select="[actions]" />
    </div>
    <div class="frame" [class.actual]="actual()" tabindex="0" role="group" [attr.aria-label]="image().name">
      <img [src]="image().dataUrl" [alt]="image().name" [attr.width]="actual() ? image().width : null" [attr.height]="actual() ? image().height : null" data-testid="image-viewer-img" />
    </div>
    <p class="facts muted" data-testid="image-facts">
      {{ size() }}
      @if (captured(); as c) {
        · {{ c }}
      }
    </p>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 6px; min-height: 0; }
    .bar { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
    .name { font-weight: 600; flex: 1; min-width: 8ch; }
    .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .small { font-size: var(--fs-xs); }
    .frame { flex: 1; min-height: 120px; max-height: 60vh; display: flex; align-items: center; justify-content: center; background: var(--bg-1); border: 1px solid var(--border); border-radius: 6px; overflow: hidden; }
    .frame:focus-visible { outline: 2px solid var(--focus-ring); }
    .frame img { max-width: 100%; max-height: 100%; object-fit: contain; display: block; }
    .frame.actual { overflow: auto; align-items: flex-start; justify-content: flex-start; }
    .frame.actual img { max-width: none; max-height: none; }
    .facts { margin: 0; font-size: var(--fs-xs); }
  `,
})
export class ImageViewer {
  readonly image = input.required<AiImage>();
  /** Shown at 100 % instead of fit to the panel. */
  protected readonly actual = signal(false);
  protected readonly size = computed(() => sizeLabel(this.image()));
  protected readonly captured = computed(() => capturedLabel(this.image()));
}
