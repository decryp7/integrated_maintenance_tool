import { ApplicationRef, Component, EnvironmentInjector } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { limitToasts, MAX_TOASTS, mountInToastStack, toastStack } from './toast-stack';

@Component({ selector: 'debrief-test-toast', template: '<div data-toast>one</div><div data-toast>two</div>' })
class TwoToasts {}

describe('toast stack', () => {
  afterEach(() => document.getElementById('debrief-toast-stack')?.remove());

  it('is one container, created on first use', () => {
    const stack = toastStack();
    expect(stack.className).toBe('debrief-toast-stack');
    expect(toastStack()).toBe(stack);
    expect(document.querySelectorAll('.debrief-toast-stack')).toHaveLength(1);
  });

  it('shows at most three toasts, the newest (last) ones (E4)', () => {
    const stack = toastStack();
    for (let i = 0; i < 5; i++) {
      const t = document.createElement('div');
      t.setAttribute('data-toast', '');
      t.textContent = `t${i}`;
      stack.append(t);
    }
    limitToasts(stack);
    const shown = [...stack.querySelectorAll('[data-toast]:not(.debrief-toast-overflow)')].map((t) => t.textContent);
    expect(shown).toEqual(['t2', 't3', 't4']);
    expect(MAX_TOASTS).toBe(3);
    stack.lastElementChild?.remove();
    stack.lastElementChild?.remove();
    limitToasts(stack);
    expect(stack.querySelectorAll('.debrief-toast-overflow')).toHaveLength(0);
  });

  it('mounts a component inside the stack and removes its host when destroyed', () => {
    const ref = mountInToastStack(TwoToasts, 'debrief-test-toast', TestBed.inject(EnvironmentInjector), TestBed.inject(ApplicationRef));
    TestBed.inject(ApplicationRef).tick();
    expect(toastStack().querySelectorAll('debrief-test-toast [data-toast]')).toHaveLength(2);
    ref.destroy();
    expect(toastStack().querySelector('debrief-test-toast')).toBeNull();
  });
});
