import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { WorkspaceStore } from '../core/workspace.store';
import { provideTinyBackend } from '../../testing/helpers';
import { EXPLORER_AUTO_HIDE_PX, NarrowWindow } from './narrow-window';

function setup() {
  TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
  return { narrow: TestBed.inject(NarrowWindow), store: TestBed.inject(WorkspaceStore) };
}

describe('NarrowWindow', () => {
  it('closes the Explorer when the window narrows and brings it back when it widens', () => {
    const { narrow, store } = setup();
    expect(store.sidebarOpen()).toBe(true);
    narrow.apply(true);
    expect(store.sidebarOpen()).toBe(false);
    narrow.apply(false);
    expect(store.sidebarOpen()).toBe(true);
  });

  it('does not reopen an Explorer the user had closed, nor close one twice', () => {
    const { narrow, store } = setup();
    store.toggleSidebar();
    narrow.apply(true);
    narrow.apply(false);
    expect(store.sidebarOpen()).toBe(false);
  });

  it('leaves the user\'s choice alone: opened by hand while narrow, it stays open when the window widens', () => {
    const { narrow, store } = setup();
    narrow.apply(true);
    store.toggleSidebar();
    narrow.apply(false);
    expect(store.sidebarOpen()).toBe(true);
    narrow.apply(false);
    expect(store.sidebarOpen()).toBe(true);
  });

  it('follows the media query', () => {
    const { narrow, store } = setup();
    const listeners: ((e: MediaQueryListEvent) => void)[] = [];
    const query = { matches: true, addEventListener: (_: string, l: (e: MediaQueryListEvent) => void) => void listeners.push(l), removeEventListener: vi.fn() };
    const matchMedia = vi.spyOn(window, 'matchMedia').mockReturnValue(query as unknown as MediaQueryList);
    narrow.start();
    narrow.start();
    expect(matchMedia).toHaveBeenCalledTimes(1);
    expect(matchMedia).toHaveBeenCalledWith(`(max-width: ${EXPLORER_AUTO_HIDE_PX - 1}px)`);
    expect(store.sidebarOpen()).toBe(false);
    listeners[0]?.({ matches: false } as MediaQueryListEvent);
    expect(store.sidebarOpen()).toBe(true);
    matchMedia.mockRestore();
  });

  it('does nothing without matchMedia', () => {
    const { narrow, store } = setup();
    const spy = vi.spyOn(window, 'matchMedia').mockImplementation(undefined as never);
    try {
      narrow.start();
      expect(store.sidebarOpen()).toBe(true);
    } finally {
      spy.mockRestore();
    }
  });
});
