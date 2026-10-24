import { afterNextRender, DestroyRef, Directive, ElementRef, inject, input } from '@angular/core';

/** Which arrow keys move inside a roving group: Up/Down, Left/Right or all four. */
export type RovingAxis = 'vertical' | 'horizontal' | 'both';

/**
 * Where a key moves the focus in a group of `count` items from `index`: the arrow keys of the axis step (no wrap, so
 * Tab-away stays the only way out), Home and End jump. Null for any other key, or when there is nowhere to go.
 */
export function rovingTarget(count: number, index: number, key: string, axis: RovingAxis): number | null {
  if (count <= 0) return null;
  const last = count - 1;
  const forward = key === 'ArrowDown' ? axis !== 'horizontal' : key === 'ArrowRight' ? axis !== 'vertical' : false;
  const backward = key === 'ArrowUp' ? axis !== 'horizontal' : key === 'ArrowLeft' ? axis !== 'vertical' : false;
  let target: number | null = null;
  if (forward) target = Math.min(last, index + 1);
  else if (backward) target = Math.max(0, index - 1);
  else if (key === 'Home') target = 0;
  else if (key === 'End') target = last;
  return target === null || target === index ? null : target;
}

/**
 * A roving tabindex group (WAI-ARIA toolbar pattern): of the host's descendants matching the selector, exactly one is
 * a Tab stop (the last one focused, else the first) and the arrow keys move between them. A toolbar of ten buttons
 * costs one Tab press instead of ten. The group follows items that come and go.
 *
 * `<div [debriefRoving]="'button'" roving-axis="horizontal">`
 */
@Directive({
  selector: '[debriefRoving]',
  host: { '(focusin)': 'onFocusIn($event)', '(keydown)': 'onKeydown($event)' },
})
export class RovingTabindex {
  /** CSS selector of the group's items. */
  readonly debriefRoving = input.required<string>();
  readonly rovingAxis = input<RovingAxis>('both');

  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
  private readonly destroyRef = inject(DestroyRef);
  private current: HTMLElement | null = null;

  constructor() {
    afterNextRender(() => {
      this.sync();
      if (typeof MutationObserver === 'undefined') return;
      const observer = new MutationObserver(() => this.sync());
      observer.observe(this.host, { childList: true, subtree: true });
      this.destroyRef.onDestroy(() => observer.disconnect());
    });
  }

  private items(): HTMLElement[] {
    return Array.from(this.host.querySelectorAll<HTMLElement>(this.debriefRoving())).filter((e) => !e.hasAttribute('disabled') && !e.closest('[hidden]'));
  }

  /** Makes the current item (kept while it exists) the one Tab stop and every other item unreachable by Tab. */
  private sync(): void {
    const items = this.items();
    if (!this.current || !items.includes(this.current)) this.current = items[0] ?? null;
    for (const item of items) {
      const tab = item === this.current ? '0' : '-1';
      if (item.getAttribute('tabindex') !== tab) item.setAttribute('tabindex', tab);
    }
  }

  protected onFocusIn(event: FocusEvent): void {
    const target = event.target as HTMLElement;
    if (this.current === target || !this.items().includes(target)) return;
    this.current = target;
    this.sync();
  }

  protected onKeydown(event: KeyboardEvent): void {
    if (event.defaultPrevented) return;
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    const items = this.items();
    const index = items.indexOf(event.target as HTMLElement);
    if (index < 0) return;
    const to = rovingTarget(items.length, index, event.key, this.rovingAxis());
    if (to === null) return;
    event.preventDefault();
    items[to]?.focus();
  }
}
