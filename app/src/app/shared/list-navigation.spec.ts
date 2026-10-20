import { Injector, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { ActiveDescendantNavigation, RovingFocusNavigation } from './list-navigation';

const key = (k: string, init: KeyboardEventInit = {}) => new KeyboardEvent('keydown', { key: k, ...init });

describe('list navigation', () => {
  const injector = () => TestBed.inject(Injector);

  it('moves through options by key name, wrapping when asked, and ignores modified keys', () => {
    const count = signal(3);
    const nav = new ActiveDescendantNavigation(count, injector(), { wrap: true, selectFirst: true });
    expect(nav.active()).toBe(0);
    expect(nav.handle(key('ArrowDown'))).toBe(true);
    expect(nav.active()).toBe(1);
    nav.handle(key('ArrowDown'));
    nav.handle(key('ArrowDown'));
    expect(nav.active()).toBe(0);
    nav.handle(key('ArrowUp'));
    expect(nav.active()).toBe(2);
    expect(nav.handle(key('ArrowDown', { altKey: true }))).toBe(false);
    expect(nav.handle(key('Home'))).toBe(false);
    expect(nav.handle(key('x'))).toBe(false);
    expect(nav.active()).toBe(2);
  });

  it('starts from the first option with selectFirst, and from none without it', () => {
    const withFirst = new ActiveDescendantNavigation(signal(4), injector(), { selectFirst: true });
    withFirst.handle(key('ArrowDown'));
    expect(withFirst.active()).toBe(1);
    const plain = new ActiveDescendantNavigation(signal(4), injector());
    expect(plain.active()).toBe(-1);
    plain.handle(key('ArrowDown'));
    expect(plain.active()).toBe(0);
  });

  it('clamps without wrap and handles Home/End and pages when enabled', () => {
    let page = 2;
    const nav = new ActiveDescendantNavigation(signal(10), injector(), { homeAndEnd: true, pageDelta: () => page });
    nav.setActive(0);
    nav.handle(key('ArrowUp'));
    expect(nav.active()).toBe(0);
    nav.handle(key('End'));
    expect(nav.active()).toBe(9);
    nav.handle(key('ArrowDown'));
    expect(nav.active()).toBe(9);
    nav.handle(key('PageUp'));
    expect(nav.active()).toBe(7);
    page = 5;
    nav.handle(key('PageUp'));
    expect(nav.active()).toBe(2);
    nav.handle(key('PageUp'));
    expect(nav.active()).toBe(0);
    nav.handle(key('Home'));
    expect(nav.active()).toBe(0);
    expect(new ActiveDescendantNavigation(signal(3), injector()).handle(key('PageDown'))).toBe(false);
  });

  it('follows the list length: clamps, clears and skips disabled options', () => {
    const count = signal(5);
    const nav = new ActiveDescendantNavigation(count, injector(), { disabled: (i) => i === 1 });
    nav.setActive(0);
    nav.handle(key('ArrowDown'));
    expect(nav.active()).toBe(2);
    nav.setActive(40);
    expect(nav.active()).toBe(4);
    count.set(2);
    TestBed.tick();
    expect(nav.active()).toBe(-1);
    count.set(0);
    nav.setActive(0);
    expect(nav.active()).toBe(-1);
    expect(nav.handle(key('ArrowDown'))).toBe(false);
  });

  it('moves focus with a roving tab stop, and sync moves the stop without focusing', () => {
    const focused: number[] = [];
    const nav = new RovingFocusNavigation(signal(3), injector(), (i) => focused.push(i), { homeAndEnd: true });
    nav.sync(1);
    expect(nav.active()).toBe(1);
    expect(focused).toEqual([]);
    nav.handle(key('ArrowDown'));
    nav.handle(key('Home'));
    nav.setActive(2);
    expect(focused).toEqual([2, 0, 2]);
    nav.sync(-1);
    expect(nav.active()).toBe(-1);
  });
});
