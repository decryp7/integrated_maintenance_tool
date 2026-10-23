import { DestroyRef, EnvironmentProviders, inject, isDevMode, makeEnvironmentProviders, provideEnvironmentInitializer } from '@angular/core';
import { isMac } from '../extensions/keybinding';
import { copyText } from '../tools/clipboard';
import { ContextMenuService } from '../ui/context-menu.service';

/**
 * Makes Debrief's windows feel like a desktop app rather than a web page (ADR 0028): the browser's own context menu, reload,
 * back/forward, page zoom, find, print, save page, view source, middle-click autoscroll and (in release builds) devtools
 * are switched off, images and links cannot be dragged out, and files dropped where nothing takes them do not navigate.
 * Right-clicking selected text opens Debrief's own menu with Copy. The Tauri shell turns off what WebView2 lets it (`native_feel.rs`);
 * this guard covers the rest and makes the browser build behave the same.
 *
 * Only the browser's default is prevented: events still reach Debrief's own handlers (shortcuts, timeline Ctrl+wheel
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

/**
 * Mouse buttons 3 and 4 are the browser's Back and Forward; the middle button (1) starts autoscroll or opens a link
 * in a new window, except in a text field (where Linux pastes the primary selection).
 */
export function shouldBlockMouseButton(button: number, editable = false): boolean {
  return button === 3 || button === 4 || (button === 1 && !editable);
}

/** Ctrl+wheel (and a trackpad pinch, reported as one) zooms the page: Debrief's own handlers still see the event. */
export function shouldBlockWheel(e: Pick<WheelEvent, 'ctrlKey'>): boolean {
  return e.ctrlKey;
}

export interface ContextMenuTarget {
  /** Debrief opened its own menu (or a handler took the event). */
  handled: boolean;
  /** A text field or editable content: its native menu (cut, copy, paste, spelling) stays. */
  editable: boolean;
  /** The right-click is on selected text: the native menu offers Copy. */
  onSelection: boolean;
  /** Shift held in a development build: the browser menu (Inspect) for developers. */
  developer: boolean;
}

/**
 * What a right-click does: `leave` it to the menu Debrief opened, keep the `native` edit menu (text fields, developers),
 * open Debrief's `copy` menu on selected text, or `block` the browser menu (it offers nothing but web-page actions).
 */
export type ContextMenuAction = 'leave' | 'native' | 'copy' | 'block';

/** The context-menu policy of ADR 0028. Pure. */
export function contextMenuAction(t: ContextMenuTarget): ContextMenuAction {
  if (t.handled) return 'leave';
  if (t.editable || t.developer) return 'native';
  return t.onSelection ? 'copy' : 'block';
}

/** A file dragged over or dropped on the window that no Debrief drop target took would navigate the window to it. */
export function shouldBlockFileDrop(t: { files: boolean; handled: boolean }): boolean {
  return t.files && !t.handled;
}

export interface DragTarget {
  /** The dragged element's tag (upper case), or null for a text selection. */
  tag: string | null;
  /** Inside a link. */
  inLink: boolean;
  /** The element opted in with `draggable="true"` (Debrief's own drag sources). */
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

function inRect(r: DOMRect, x: number, y: number): boolean {
  return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
}

/**
 * The selected text when the right-click at (`x`, `y`) lands on, or inside, the selection. Where the browser lays text
 * out, that is the pointer being over one of the range's rectangles; without layout (tests) the target must be inside
 * the range's common ancestor, so a click on an ancestor such as `body` or on an empty area never counts.
 */
function selectedTextAt(el: Element | null, selection: Selection | null, x: number, y: number): string | null {
  if (!el || !selection || selection.isCollapsed) return null;
  const text = selection.toString();
  if (!text.trim()) return null;
  for (let i = 0; i < selection.rangeCount; i++) {
    const range = selection.getRangeAt(i);
    if (!range.intersectsNode(el)) continue;
    const rects = typeof range.getClientRects === 'function' ? Array.from(range.getClientRects()) : [];
    if (rects.length > 0) {
      if (rects.some((r) => inRect(r, x, y))) return text;
      continue;
    }
    const common = range.commonAncestorContainer;
    const commonEl = common instanceof Element ? common : common.parentElement;
    if (commonEl?.contains(el)) return text;
  }
  return null;
}

/** What the guard asks of the app. */
export interface FeelHooks {
  /** Opens Debrief's menu with Copy for `text` at the pointer. Without it, the native menu stays on selected text. */
  copyMenu?: (x: number, y: number, text: string) => void;
}

/** Installs the guard on a window; returns the function that removes it. */
export function installNativeFeel(win: Window, o: FeelOptions, hooks: FeelHooks = {}): () => void {
  const key = (e: KeyboardEvent) => {
    if (!e.defaultPrevented && shouldBlockKey(e, o)) e.preventDefault();
  };
  const mouse = (e: MouseEvent) => {
    if (shouldBlockMouseButton(e.button, isEditable(elementOf(e.target)))) e.preventDefault();
  };
  const wheel = (e: WheelEvent) => {
    if (!e.defaultPrevented && shouldBlockWheel(e)) e.preventDefault();
  };
  const menu = (e: MouseEvent) => {
    const el = elementOf(e.target);
    const selected = selectedTextAt(el, win.getSelection(), e.clientX, e.clientY);
    const action = contextMenuAction({
      handled: e.defaultPrevented,
      editable: isEditable(el),
      onSelection: selected !== null,
      developer: !o.release && e.shiftKey,
    });
    if (action === 'block') e.preventDefault();
    if (action === 'copy' && selected !== null && hooks.copyMenu) {
      e.preventDefault();
      hooks.copyMenu(e.clientX, e.clientY, selected);
    }
  };
  const files = (e: DragEvent) => {
    const dropped = !!e.dataTransfer && Array.from(e.dataTransfer.types ?? []).includes('Files');
    if (!shouldBlockFileDrop({ files: dropped, handled: e.defaultPrevented })) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'none';
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
  win.addEventListener('dragover', files);
  win.addEventListener('drop', files);
  return () => {
    win.removeEventListener('keydown', key);
    win.removeEventListener('mousedown', mouse);
    win.removeEventListener('mouseup', mouse);
    win.removeEventListener('auxclick', mouse);
    win.removeEventListener('wheel', wheel);
    win.removeEventListener('contextmenu', menu);
    win.removeEventListener('dragstart', drag);
    win.removeEventListener('dragover', files);
    win.removeEventListener('drop', files);
  };
}

/** Installs the guard once per window (main window and pop-outs share the app config). */
export function provideNativeFeel(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideEnvironmentInitializer(() => {
      if (typeof window === 'undefined') return;
      const menu = inject(ContextMenuService);
      const copyMenu = (x: number, y: number, text: string) =>
        menu.open(x, y, [{ label: $localize`:Context menu item on selected text@@nativeFeel.copy:Copy`, run: () => void copyText(text) }]);
      const remove = installNativeFeel(window, { release: !isDevMode(), mac: isMac() }, { copyMenu });
      inject(DestroyRef).onDestroy(remove);
    }),
  ]);
}
