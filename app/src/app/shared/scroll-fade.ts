import { afterNextRender, DestroyRef, Directive, ElementRef, inject } from '@angular/core';

/** Whether a scroll box has content below its visible part (a pixel of slack absorbs rounding). */
export function hasMoreBelow(scrollHeight: number, clientHeight: number, scrollTop: number): boolean {
  return scrollHeight - clientHeight - scrollTop > 1;
}

/**
 * Marks a scrolling list that continues below its edge: sets `data-more-below` on the host while content is cut off, so
 * the stylesheet can fade the bottom edge instead of showing a half row that looks like a rendering bug. Follows scrolling,
 * resizing and items that come and go.
 *
 * `<div class="list" debriefScrollFade>` with `.list[data-more-below] { mask-image: … }`
 */
@Directive({ selector: '[debriefScrollFade]' })
export class ScrollFade {
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;

  constructor() {
    const destroyRef = inject(DestroyRef);
    afterNextRender(() => {
      const update = () => {
        const more = hasMoreBelow(this.host.scrollHeight, this.host.clientHeight, this.host.scrollTop);
        if (more) this.host.setAttribute('data-more-below', '');
        else this.host.removeAttribute('data-more-below');
      };
      this.host.addEventListener('scroll', update, { passive: true });
      const resize = typeof ResizeObserver === 'function' ? new ResizeObserver(update) : null;
      resize?.observe(this.host);
      const mutation = typeof MutationObserver === 'function' ? new MutationObserver(update) : null;
      mutation?.observe(this.host, { childList: true, subtree: true });
      update();
      destroyRef.onDestroy(() => {
        this.host.removeEventListener('scroll', update);
        resize?.disconnect();
        mutation?.disconnect();
      });
    });
  }
}
