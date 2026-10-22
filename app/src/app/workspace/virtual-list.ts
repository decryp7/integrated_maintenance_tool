import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, contentChild, input, signal, TemplateRef } from '@angular/core';
import { makeGeometry, visibleWindow } from '../grid/scroll-map';

const OVERSCAN = 6;

/** What an item template receives: `let-item` is the item, `let-i="index"` its position. */
export interface VirtualItemContext<T> {
  $implicit: T;
  index: number;
}

/**
 * A scrolling list that renders only the rows in view (plus a little overscan), so a list of tens of
 * thousands of values costs a few dozen DOM nodes. Rows have a fixed height. Pass the row template as
 * the only content: `<debrief-virtual-list [items]="x" label="Values"><ng-template let-item>…</ng-template></debrief-virtual-list>`.
 */
@Component({
  selector: 'debrief-virtual-list',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [NgTemplateOutlet],
  template: `
    <div class="vl" role="list" tabindex="0" [attr.aria-label]="label()" [style.height.px]="height()" (scroll)="onScroll($event)">
      <div class="sizer" [style.height.px]="geometry().virtualH">
        @for (v of rows(); track v.index) {
          <div class="vi" role="listitem" [attr.aria-posinset]="v.index + 1" [attr.aria-setsize]="items().length" [style.top.px]="v.index * itemHeight()" [style.height.px]="itemHeight()">
            <ng-container *ngTemplateOutlet="template(); context: { $implicit: v.item, index: v.index }" />
          </div>
        }
      </div>
    </div>
  `,
  styles: `
    :host { display: block; min-width: 0; }
    .vl { overflow-y: auto; overflow-x: hidden; position: relative; }
    .vl:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }
    .sizer { position: relative; }
    .vi { position: absolute; left: 0; right: 0; display: flex; align-items: center; }
  `,
})
export class VirtualList<T> {
  readonly items = input.required<readonly T[]>();
  readonly itemHeight = input(24);
  /** Tallest the list grows before it scrolls. */
  readonly maxHeight = input(240);
  readonly label = input.required<string>();
  protected readonly template = contentChild.required(TemplateRef);

  private readonly scrollTop = signal(0);
  protected readonly height = computed(() => Math.min(this.items().length * this.itemHeight(), this.maxHeight()));
  protected readonly geometry = computed(() => makeGeometry(this.items().length, this.itemHeight(), this.height()));
  protected readonly rows = computed(() => {
    const g = this.geometry();
    const { first, last } = visibleWindow(g, this.scrollTop(), OVERSCAN);
    const items = this.items();
    const out: { index: number; item: T }[] = [];
    for (const [k, item] of items.slice(first, last).entries()) out.push({ index: first + k, item });
    return out;
  });

  protected onScroll(e: Event): void {
    this.scrollTop.set((e.target as HTMLElement).scrollTop);
  }
}
