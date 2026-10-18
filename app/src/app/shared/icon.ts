import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { MatIcon } from '@angular/material/icon';
import { ImtIconSet } from './material/imt-icons';

/**
 * An icon from IMT's themed SVG set, rendered by `<mat-icon>` from literals registered in the
 * `imt` namespace. Decorative (hidden from assistive tech) unless `label` is set.
 */
@Component({
  selector: 'imt-icon',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MatIcon],
  template: `<mat-icon
    [svgIcon]="svgIcon()"
    [style.width.px]="size()"
    [style.height.px]="size()"
    [style.stroke-width]="stroke()"
    [attr.aria-hidden]="label() ? null : 'true'"
    [attr.aria-label]="label() || null"
    [attr.role]="label() ? 'img' : null"
  />`,
  styles: `
    :host {
      display: inline-flex;
      flex: none;
    }
    mat-icon {
      display: block;
      line-height: 0;
      font-size: inherit;
    }
  `,
})
export class Icon {
  private readonly icons = inject(ImtIconSet);

  readonly name = input.required<string>();
  readonly size = input(16);
  readonly stroke = input(1.7);
  /** Accessible name; when empty the icon is decorative. */
  readonly label = input('');
  protected readonly svgIcon = computed(() => this.icons.svgIcon(this.name()));
}
