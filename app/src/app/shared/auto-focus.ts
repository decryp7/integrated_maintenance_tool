import { afterNextRender, Directive, ElementRef, inject } from '@angular/core';

/**
 * Focuses the host once it has been rendered, e.g. an input that appears when the user starts an inline
 * rename. Replaces the `autofocus` attribute, which browsers apply only once per document load.
 */
@Directive({ selector: '[debriefAutoFocus]' })
export class AutoFocus {
  private readonly el = inject<ElementRef<HTMLElement>>(ElementRef);

  constructor() {
    afterNextRender(() => this.el.nativeElement.focus());
  }
}
