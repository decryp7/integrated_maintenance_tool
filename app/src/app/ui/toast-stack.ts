import { ApplicationRef, ComponentRef, createComponent, EnvironmentInjector, Type } from '@angular/core';

/** The most toasts the stack shows at once; older ones wait (hidden) until a newer one goes. */
export const MAX_TOASTS = 3;
/** Marks one toast inside a stacked component (a component may show several, e.g. live alerts). */
export const TOAST_ATTR = 'data-toast';
const STACK_ID = 'imt-toast-stack';
const OVERFLOW_CLASS = 'imt-toast-overflow';

/** Hides every toast but the last `max` in document order (the bottom of the stack is the newest). */
export function limitToasts(stack: HTMLElement, max = MAX_TOASTS): void {
  const toasts = [...stack.querySelectorAll<HTMLElement>(`[${TOAST_ATTR}]`)];
  toasts.forEach((t, i) => t.classList.toggle(OVERFLOW_CLASS, i < toasts.length - max));
}

/**
 * The one bottom-right stack every toast of the main window goes into (errors, notices, live alerts, error
 * snapshots), so they sit one above the other with a gap instead of on top of each other. Created on first use;
 * styled in `styles.scss` (`.imt-toast-stack`).
 */
export function toastStack(doc: Document = document): HTMLElement {
  let stack = doc.getElementById(STACK_ID);
  if (stack) return stack;
  stack = doc.createElement('div');
  stack.id = STACK_ID;
  stack.className = 'imt-toast-stack';
  doc.body.append(stack);
  const el = stack;
  new MutationObserver(() => limitToasts(el)).observe(el, { childList: true, subtree: true });
  return stack;
}

/** Creates `component` inside the toast stack (as `<tag>`) and attaches it to change detection. */
export function mountInToastStack<T>(component: Type<T>, tag: string, injector: EnvironmentInjector, appRef: ApplicationRef): ComponentRef<T> {
  const host = document.createElement(tag);
  toastStack().append(host);
  const ref = createComponent(component, { environmentInjector: injector, hostElement: host });
  appRef.attachView(ref.hostView);
  ref.onDestroy(() => host.remove());
  return ref;
}
