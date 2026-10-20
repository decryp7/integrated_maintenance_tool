import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { Swatch } from './source-colors';

/**
 * A source or instance colour dot: filled (solid), a dashed ring or a dotted ring, matching the row stripe's pattern.
 * It carries its legend as tooltip and accessible name.
 */
@Component({
  selector: 'imt-source-swatch',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { role: 'img', '[attr.aria-label]': 'label()', '[attr.title]': 'label()', '[attr.data-slot]': 'swatch().slot', '[attr.data-pattern]': 'swatch().pattern' },
  template: `
    <svg [attr.width]="size()" [attr.height]="size()" viewBox="0 0 10 10" aria-hidden="true">
      @if (swatch().pattern === 'solid') {
        <circle cx="5" cy="5" r="4.5" [attr.fill]="swatch().color" />
      } @else {
        <circle cx="5" cy="5" r="3.75" fill="none" stroke-width="2.5" [attr.stroke]="swatch().color" [attr.stroke-dasharray]="dash()" />
      }
    </svg>
  `,
  styles: `
    :host { display: inline-flex; flex: none; align-items: center; line-height: 0; }
  `,
})
export class SourceSwatch {
  readonly swatch = input.required<Swatch>();
  readonly label = input.required<string>();
  readonly size = input(10);
  protected readonly dash = computed(() => (this.swatch().pattern === 'dashed' ? '4 2' : '1.6 1.6'));
}
