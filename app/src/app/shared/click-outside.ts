import { Directive, ElementRef, inject, output } from '@angular/core';

/** Emits when a pointerdown or Escape happens outside the host (for popovers). */
@Directive({
  selector: '[imtClickOutside]',
  host: {
    '(document:pointerdown)': 'onPointer($event)',
    '(document:keydown.escape)': 'imtClickOutside.emit()',
  },
})
export class ClickOutside {
  readonly imtClickOutside = output<void>();
  private readonly el = inject<ElementRef<HTMLElement>>(ElementRef);

  protected onPointer(event: PointerEvent): void {
    if (!this.el.nativeElement.contains(event.target as Node)) this.imtClickOutside.emit();
  }
}
