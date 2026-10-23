import { Directive, ElementRef, inject, output } from '@angular/core';

/** Emits when a pointerdown or Escape happens outside the host (for popovers). */
@Directive({
  selector: '[debriefClickOutside]',
  host: {
    '(document:pointerdown)': 'onPointer($event)',
    '(document:keydown.escape)': 'debriefClickOutside.emit()',
  },
})
export class ClickOutside {
  readonly debriefClickOutside = output<void>();
  private readonly el = inject<ElementRef<HTMLElement>>(ElementRef);

  protected onPointer(event: PointerEvent): void {
    if (!this.el.nativeElement.contains(event.target as Node)) this.debriefClickOutside.emit();
  }
}
