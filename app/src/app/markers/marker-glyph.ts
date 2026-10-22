import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, input, viewChild } from '@angular/core';
import { MarkerIcon } from '../shared/marker-icon';
import { badgeFontSize, GlyphSpec, resolveGlyph, sanitizeSvg, shapeGeometry } from './glyph-model';
import { MarkerStyles } from './marker-styles';
import { GlyphStyle } from './markers.types';

/**
 * Draws a marker: a generated badge (1-3 characters in a circle, square, diamond or hexagon), a
 * built-in or custom icon, or the kind's default shape. Give `kind` to look the style up in the
 * workspace summary, or `glyph` to draw an explicit style (editor previews).
 *
 * Decorative by default (the label sits beside it); pass `label` to expose it as an image.
 */
@Component({
  selector: 'debrief-marker-glyph',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MarkerIcon],
  host: {
    '[style.width.px]': 'size()',
    '[style.height.px]': 'size()',
    '[style.color]': 'inkColor()',
    '[attr.role]': 'label() ? "img" : null',
    '[attr.aria-label]': 'label() || null',
    '[attr.aria-hidden]': 'label() ? null : "true"',
    '[attr.data-glyph]': 'spec().type',
  },
  template: `
    @switch (spec().type) {
      @case ('shape') {
        <debrief-marker-icon [kind]="shapeKind()" [color]="shapeColor()" aria-hidden="true" />
      }
      @case ('badge') {
        <svg viewBox="0 0 24 24" width="100%" height="100%" focusable="false" aria-hidden="true">
          @switch (badge()?.geo?.tag) {
            @case ('circle') { <circle [attr.cx]="badge()!.geo.attrs['cx']" [attr.cy]="badge()!.geo.attrs['cy']" [attr.r]="badge()!.geo.attrs['r']" [attr.fill]="badge()!.fill" class="outline" /> }
            @case ('rect') { <rect [attr.x]="badge()!.geo.attrs['x']" [attr.y]="badge()!.geo.attrs['y']" [attr.width]="badge()!.geo.attrs['width']" [attr.height]="badge()!.geo.attrs['height']" [attr.rx]="badge()!.geo.attrs['rx']" [attr.fill]="badge()!.fill" class="outline" /> }
            @default { <polygon [attr.points]="badge()!.geo.attrs['points']" [attr.fill]="badge()!.fill" class="outline" /> }
          }
        </svg>
        <span class="badge-text mono" [attr.data-t]="badge()!.text" [style.color]="badge()!.ink" [style.font-size.px]="badgeFont()"></span>
      }
      @case ('icon') {
        <svg viewBox="0 0 24 24" width="100%" height="100%" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" focusable="false" aria-hidden="true">
          <path [attr.d]="iconPath()" />
        </svg>
      }
      @case ('svg') {
        <span class="slot" #slot></span>
      }
      @case ('png') {
        <img [src]="pngSrc()" alt="" draggable="false" />
      }
    }
  `,
  styles: `
    :host { position: relative; display: inline-flex; align-items: center; justify-content: center; flex: none; line-height: 0; }
    /* The text is drawn from an attribute so assistive tooling does not read it twice (it is decorative). */
    .badge-text { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; font-family: var(--font-mono), monospace; font-weight: 700; line-height: 1; pointer-events: none; }
    .badge-text::after { content: attr(data-t); }
    /* A ring in the surface colour separates the glyph from what it overlaps (density, runs, other glyphs). */
    .outline { stroke: var(--ring); stroke-width: 1.5; paint-order: stroke; }
    .slot, img { display: block; width: 100%; height: 100%; object-fit: contain; }
  `,
})
export class MarkerGlyph {
  private readonly styles = inject(MarkerStyles);

  readonly kind = input<string>('');
  /** Explicit style; wins over the `kind` lookup. */
  readonly glyph = input<GlyphStyle | null>(null);
  readonly size = input(14);
  readonly label = input('');

  private readonly slot = viewChild<ElementRef<HTMLElement>>('slot');

  protected readonly spec = computed<GlyphSpec>(() => {
    const explicit = this.glyph();
    return resolveGlyph(explicit ?? this.styles.style(this.kind()), this.styles.icons());
  });
  protected readonly inkColor = computed(() => {
    const s = this.spec();
    return s.type === 'svg' || s.type === 'icon' ? s.color : null;
  });
  protected readonly badge = computed(() => {
    const s = this.spec();
    return s.type === 'badge' ? { text: s.text, fill: s.fill, ink: s.ink, geo: shapeGeometry(s.shape), font: badgeFontSize([...s.text].length, s.shape) } : null;
  });
  protected readonly badgeFont = computed(() => ((this.badge()?.font ?? 0) * this.size()) / 24);
  protected readonly shapeKind = computed(() => {
    const s = this.spec();
    return s.type === 'shape' ? s.kind : '';
  });
  protected readonly shapeColor = computed(() => {
    const s = this.spec();
    return s.type === 'shape' ? s.color : null;
  });
  protected readonly iconPath = computed(() => {
    const s = this.spec();
    return s.type === 'icon' ? s.d : '';
  });
  protected readonly pngSrc = computed(() => {
    const s = this.spec();
    return s.type === 'png' ? s.src : '';
  });

  constructor() {
    effect(() => {
      const host = this.slot()?.nativeElement;
      const s = this.spec();
      if (!host) return;
      const node = s.type === 'svg' ? sanitizeSvg(s.markup, host.ownerDocument) : null;
      host.replaceChildren(...(node ? [node] : []));
    });
  }
}
