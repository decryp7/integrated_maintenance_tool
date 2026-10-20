import { DestroyRef, EnvironmentProviders, inject, isDevMode, makeEnvironmentProviders, provideEnvironmentInitializer } from '@angular/core';
import { isMac } from '../extensions/keybinding';

/**
 * Makes IMT's windows feel like a desktop app rather than a web page: the browser's own context menu, reload,
 * back/forward, page zoom, find, print, save page, view source and (in release builds) devtools are switched off,
 * and images and links cannot be dragged out. The Tauri shell turns off what WebView2 lets it (`native_feel.rs`);
 * this guard covers the rest and makes the browser build behave the same.
 *
 * Only the browser's default is prevented: events still reach IMT's own handlers (shortcuts, timeline Ctrl+wheel
 * zoom, context menus, drag-and-drop), which run first because the guard listens on the window in the bubble phase.
 */

export interface FeelOptions {
  /** A release build: devtools keys are blocked too (development builds keep them and hard reload). */
  release: boolean;
  /** "Mod" is Cmd. */
  mac: boolean;
}

export type KeyLike = Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'> & { code?: string };

/** What a browser would do with a key. */
export type BrowserKeyAction = 'reload' | 'hard-reload' | 'navigate' | 'zoom' | 'find' | 'print' | 'save' | 'source' | 'devtools';

const ZOOM_KEYS = new Set(['=', '+', '-', '_', '0']);
const ZOOM_CODES = new Set(['Equal', 'Minus', 'Digit0', 'NumpadAdd', 'NumpadSubtract', 'Numpad0']);

/** The browser feature a key combination triggers, if any. Pure. */
export function browserKeyAction(e: KeyLike, mac: boolean): BrowserKeyAction | null {
  const mod = mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if (key === 'F5' || key === 'BrowserRefresh') return e.shiftKey || e.ctrlKey ? 'hard-reload' : 'reload';
  if (key === 'BrowserBack' || key === 'BrowserForward') return 'navigate';
  if (e.altKey && !e.ctrlKey && !e.metaKey && (key === 'ArrowLeft' || key === 'ArrowRight')) return 'navigate';
  if (key === 'F12') return 'devtools';
  if (key === 'F3' || key === 'BrowserSearch') return 'find';
  if (!mod) return null;
  if (ZOOM_KEYS.has(key) || (e.code !== undefined && ZOOM_CODES.has(e.code))) return e.altKey ? null : 'zoom';
  // Devtools: Ctrl+Shift+I/J/C, or Cmd+Option+I/J/C on macOS.
  if ((mac ? e.altKey : e.shiftKey) && (key === 'i' || key === 'j' || key === 'c')) return 'devtools';
  if (e.altKey) return null;
  switch (key) {
    case 'r':
      return e.shiftKey ? 'hard-reload' : 'reload';
    case 'f':
    case 'g':
      return 'find';
    case 'p':
      return e.shiftKey ? null : 'print';
    case 's':
      return e.shiftKey ? null : 'save';
    case 'u':
      return e.shiftKey ? null : 'source';
    case '[':
    case ']':
      return mac ? 'navigate' : null;
    default:
      return null;
  }
}

/** Should the browser's default for this key be prevented? Development builds keep devtools and hard reload. */
export function shouldBlockKey(e: KeyLike, o: FeelOptions): boolean {
  const action = browserKeyAction(e, o.mac);
  if (!action) return false;
  if (!o.release && (action === 'devtools' || action === 'hard-reload')) return false;
  return true;
}

/** Mouse buttons 3 and 4 are the browser's Back and Forward. */
export function shouldBlockMouseButton(button: number): boolean {
  return button === 3 || button === 4;
}

/** Ctrl+wheel (and a trackpad pinch, reported as one) zooms the page: IMT's own handlers still see the event. */
export function shouldBlockWheel(e: Pick<WheelEvent, 'ctrlKey'>): boolean {
  return e.ctrlKey;
}

export interface ContextMenuTarget {
  /** IMT opened its own menu (or a handler took the event). */
  handled: boolean;
  /** A text field or editable content: its native menu (cut, copy, paste, spelling) stays. */
  editable: boolean;
  /** The right-click is on selected text: the native menu offers Copy. */
  onSelection: boolean;
  /** Shift held in a development build: the browser menu (Inspect) for developers. */
  developer: boolean;
}

/** Should the browser's context menu be suppressed? Only where it offers nothing but web-page actions. */
export function shouldBlockContextMenu(t: ContextMenuTarget): boolean {
  return !t.handled && !t.editable && !t.onSelection && !t.developer;
}

export interface DragTarget {
  /** The dragged element's tag (upper case), or null for a text selection. */
  tag: string | null;
  /** Inside a link. */
  inLink: boolean;
  /** The element opted in with `draggable="true"` (IMT's own drag sources). */
  draggable: boolean;
}

/** Images and links are draggable by default and would leave the app as files or URLs. */
export function shouldBlockDrag(t: DragTarget): boolean {
  if (t.draggable || !t.tag) return false;
  return t.tag === 'IMG' || t.tag === 'A' || t.inLink;
}

const TEXT_INPUTS = new Set(['text', 'search', 'url', 'email', 'password', 'number', 'tel', '']);

/** Is the element a text field or editable content? */
export function isEditable(el: Element | null): boolean {
  if (!el) return false;
  if (el instanceof HTMLTextAreaElement) return !el.disabled;
  if (el instanceof HTMLInputElement) return !el.disabled && TEXT_INPUTS.has(el.type);
  return el instanceof HTMLElement && el.isContentEditable === true;
}

function elementOf(target: EventTarget | null): Element | null {
  if (target instanceof Element) return target;
  return target instanceof Node ? target.parentElement : null;
}

function onSelection(el: Element | null, selection: Selection | null): boolean {
  if (!el || !selection || selection.isCollapsed || !selection.toString().trim()) return false;
  for (let i = 0; i < selection.rangeCount; i++) if (selection.getRangeAt(i).intersectsNode(el)) return true;
  return false;
}

/** Installs the guard on a window; returns the function that removes it. */
export function installNativeFeel(win: Window, o: FeelOptions): () => void {
  const key = (e: KeyboardEvent) => {
    if (!e.defaultPrevented && shouldBlockKey(e, o)) e.preventDefault();
  };
  const mouse = (e: MouseEvent) => {
    if (shouldBlockMouseButton(e.button)) e.preventDefault();
  };
  const wheel = (e: WheelEvent) => {
    if (!e.defaultPrevented && shouldBlockWheel(e)) e.preventDefault();
  };
  const menu = (e: MouseEvent) => {
    const el = elementOf(e.target);
    const block = shouldBlockContextMenu({
      handled: e.defaultPrevented,
      editable: isEditable(el),
      onSelection: onSelection(el, win.getSelection()),
      developer: !o.release && e.shiftKey,
    });
    if (block) e.preventDefault();
  };
  const drag = (e: DragEvent) => {
    const el = e.target instanceof Element ? e.target : null;
    const block = shouldBlockDrag({ tag: el?.tagName ?? null, inLink: !!el?.closest('a[href]'), draggable: el?.getAttribute('draggable') === 'true' });
    if (block) e.preventDefault();
  };
  const passive = { passive: false } as const;
  win.addEventListener('keydown', key);
  win.addEventListener('mousedown', mouse);
  win.addEventListener('mouseup', mouse);
  win.addEventListener('auxclick', mouse);
  win.addEventListener('wheel', wheel, passive);
  win.addEventListener('contextmenu', menu);
  win.addEventListener('dragstart', drag);
  return () => {
    win.removeEventListener('keydown', key);
    win.removeEventListener('mousedown', mouse);
    win.removeEventListener('mouseup', mouse);
    win.removeEventListener('auxclick', mouse);
    win.removeEventListener('wheel', wheel);
    win.removeEventListener('contextmenu', menu);
    win.removeEventListener('dragstart', drag);
  };
}

/** Installs the guard once per window (main window and pop-outs share the app config). */
export function provideNativeFeel(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideEnvironmentInitializer(() => {
      if (typeof window === 'undefined') return;
      const remove = installNativeFeel(window, { release: !isDevMode(), mac: isMac() });
      inject(DestroyRef).onDestroy(remove);
    }),
  ]);
}
