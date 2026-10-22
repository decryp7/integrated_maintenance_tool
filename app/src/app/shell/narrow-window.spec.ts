import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { WorkspaceStore } from '../core/workspace.store';
import { provideTinyBackend } from '../../testing/helpers';
import { DockService } from '../dock/dock.service';
import { DOCKS_AUTO_HIDE_PX, EXPLORER_AUTO_HIDE_PX, NarrowWindow } from './narrow-window';

function setup() {
  TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
  return { narrow: TestBed.inject(NarrowWindow), store: TestBed.inject(WorkspaceStore) };
}

describe('NarrowWindow: side docks (H1)', () => {
  it('auto-hides the docked tool windows when the window narrows and pins them back when it widens', () => {
    const { narrow } = setup();
    const dock = TestBed.inject(DockService);
    const hide = vi.spyOn(dock, 'autoHideAll').mockReturnValue(['markers', 'notes']);
    const pin = vi.spyOn(dock, 'pinIds').mockImplementation(() => undefined);
    narrow.applyDocks(true);
    expect(hide).toHaveBeenCalledTimes(1);
    expect(pin).not.toHaveBeenCalled();
    narrow.applyDocks(false);
    expect(pin).toHaveBeenCalledWith(['markers', 'notes']);
    // Nothing is pinned twice: the list was handed over.
    narrow.applyDocks(false);
    expect(pin).toHaveBeenLastCalledWith([]);
  });

  it('keeps what it hid across repeated narrowing', () => {
    const { narrow } = setup();
    const dock = TestBed.inject(DockService);
    vi.spyOn(dock, 'autoHideAll').mockReturnValueOnce(['a']).mockReturnValueOnce(['b']);
    const pin = vi.spyOn(dock, 'pinIds').mockImplementation(() => undefined);
    narrow.applyDocks(true);
    narrow.applyDocks(true);
    narrow.applyDocks(false);
    expect(pin).toHaveBeenCalledWith(['a', 'b']);
  });
});

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
    expect(matchMedia).toHaveBeenCalledTimes(2);
    expect(matchMedia).toHaveBeenCalledWith(`(max-width: ${DOCKS_AUTO_HIDE_PX - 1}px)`);
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
