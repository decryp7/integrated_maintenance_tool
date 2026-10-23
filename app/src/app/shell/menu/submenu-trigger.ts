import { CdkMenuTrigger } from '@angular/cdk/menu';
import { DestroyRef, Directive, ElementRef, inject } from '@angular/core';

/**
 * Keeps a menu open when its trigger is clicked right after hovering opened it. CDK opens menus on hover (submenus,
 * and the next menu of the bar while one is open) and toggles them on click, so the usual "hover, then click" closed
 * the menu again. A menu opened by a click still closes on the next click. Capture listeners on the target run
 * before CDK's own.
 */
@Directive({ selector: '[debriefMenuTrigger]' })
export class MenuTriggerGuard {
  constructor() {
    const trigger = inject(CdkMenuTrigger);
    const el = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
    let clicking = false;
    let openedByHover = false;
    const onClick = (e: MouseEvent) => {
      if (openedByHover && trigger.isOpen()) {
        openedByHover = false;
        e.stopImmediatePropagation();
        return;
      }
      clicking = true;
      queueMicrotask(() => (clicking = false));
    };
    const opened = trigger.opened.subscribe(() => (openedByHover = !clicking));
    const closed = trigger.closed.subscribe(() => (openedByHover = false));
    el.addEventListener('click', onClick, true);
    inject(DestroyRef).onDestroy(() => {
      el.removeEventListener('click', onClick, true);
      opened.unsubscribe();
      closed.unsubscribe();
    });
  }
}
