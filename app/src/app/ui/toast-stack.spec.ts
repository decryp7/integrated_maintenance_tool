import { ApplicationRef, Component, EnvironmentInjector } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { limitToasts, MAX_TOASTS, mountInToastStack, toastStack } from './toast-stack';

@Component({ selector: 'imt-test-toast', template: '<div data-toast>one</div><div data-toast>two</div>' })
class TwoToasts {}

describe('toast stack', () => {
  afterEach(() => document.getElementById('imt-toast-stack')?.remove());

  it('is one container, created on first use', () => {
    const stack = toastStack();
    expect(stack.className).toBe('imt-toast-stack');
    expect(toastStack()).toBe(stack);
    expect(document.querySelectorAll('.imt-toast-stack')).toHaveLength(1);
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
    const shown = [...stack.querySelectorAll('[data-toast]:not(.imt-toast-overflow)')].map((t) => t.textContent);
    expect(shown).toEqual(['t2', 't3', 't4']);
    expect(MAX_TOASTS).toBe(3);
    stack.lastElementChild?.remove();
    stack.lastElementChild?.remove();
    limitToasts(stack);
    expect(stack.querySelectorAll('.imt-toast-overflow')).toHaveLength(0);
  });

  it('mounts a component inside the stack and removes its host when destroyed', () => {
    const ref = mountInToastStack(TwoToasts, 'imt-test-toast', TestBed.inject(EnvironmentInjector), TestBed.inject(ApplicationRef));
    TestBed.inject(ApplicationRef).tick();
    expect(toastStack().querySelectorAll('imt-test-toast [data-toast]')).toHaveLength(2);
    ref.destroy();
    expect(toastStack().querySelector('imt-test-toast')).toBeNull();
  });
});
