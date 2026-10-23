import { TestBed } from '@angular/core/testing';
import { ContextMenuService } from '../ui/context-menu.service';
import { EnvironmentInjector, createEnvironmentInjector } from '@angular/core';
import {
  browserKeyAction,
  contextMenuAction,
  installNativeFeel,
  isEditable,
  KeyLike,
  provideNativeFeel,
  shouldBlockDrag,
  shouldBlockFileDrop,
  shouldBlockKey,
  shouldBlockMouseButton,
  shouldBlockWheel,
} from './native-feel';

const k = (key: string, mods: Partial<KeyLike> = {}): KeyLike => ({ key, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods });
const RELEASE = { release: true, mac: false };
const DEV = { release: false, mac: false };
const MAC = { release: true, mac: true };

describe('native feel: key decisions', () => {
  it('classifies the browser keys on Windows and Linux', () => {
    expect(browserKeyAction(k('F5'), false)).toBe('reload');
    expect(browserKeyAction(k('F5', { shiftKey: true }), false)).toBe('hard-reload');
    expect(browserKeyAction(k('F5', { ctrlKey: true }), false)).toBe('hard-reload');
    expect(browserKeyAction(k('BrowserRefresh'), false)).toBe('reload');
    expect(browserKeyAction(k('r', { ctrlKey: true }), false)).toBe('reload');
    expect(browserKeyAction(k('R', { ctrlKey: true, shiftKey: true }), false)).toBe('hard-reload');
    expect(browserKeyAction(k('ArrowLeft', { altKey: true }), false)).toBe('navigate');
    expect(browserKeyAction(k('ArrowRight', { altKey: true }), false)).toBe('navigate');
    expect(browserKeyAction(k('BrowserBack'), false)).toBe('navigate');
    expect(browserKeyAction(k('BrowserForward'), false)).toBe('navigate');
    for (const key of ['=', '+', '-', '_', '0']) expect(browserKeyAction(k(key, { ctrlKey: true }), false)).toBe('zoom');
    expect(browserKeyAction(k('Add', { ctrlKey: true, code: 'NumpadAdd' }), false)).toBe('zoom');
    expect(browserKeyAction(k('f', { ctrlKey: true }), false)).toBe('find');
    expect(browserKeyAction(k('g', { ctrlKey: true }), false)).toBe('find');
    expect(browserKeyAction(k('F3'), false)).toBe('find');
    expect(browserKeyAction(k('p', { ctrlKey: true }), false)).toBe('print');
    expect(browserKeyAction(k('s', { ctrlKey: true }), false)).toBe('save');
    expect(browserKeyAction(k('u', { ctrlKey: true }), false)).toBe('source');
    expect(browserKeyAction(k('F12'), false)).toBe('devtools');
    for (const key of ['I', 'J', 'C']) expect(browserKeyAction(k(key, { ctrlKey: true, shiftKey: true }), false)).toBe('devtools');
  });

  it('uses Cmd on macOS', () => {
    expect(browserKeyAction(k('r', { metaKey: true }), true)).toBe('reload');
    expect(browserKeyAction(k('r', { ctrlKey: true }), true)).toBeNull();
    expect(browserKeyAction(k('i', { metaKey: true, altKey: true }), true)).toBe('devtools');
    expect(browserKeyAction(k('[', { metaKey: true }), true)).toBe('navigate');
    expect(browserKeyAction(k('[', { ctrlKey: true }), false)).toBeNull();
    expect(browserKeyAction(k('=', { metaKey: true }), true)).toBe('zoom');
  });

  it('leaves editing, Debrief shortcuts and plain typing alone', () => {
    for (const e of [
      k('c', { ctrlKey: true }),
      k('v', { ctrlKey: true }),
      k('x', { ctrlKey: true }),
      k('a', { ctrlKey: true }),
      k('z', { ctrlKey: true }),
      k('k', { ctrlKey: true }),
      k('S', { ctrlKey: true, shiftKey: true }),
      k('P', { ctrlKey: true, shiftKey: true }),
      k('U', { ctrlKey: true, shiftKey: true }),
      k('o', { ctrlKey: true, altKey: true }),
      k('-', { altKey: true }),
      k('-', { ctrlKey: true, altKey: true }),
      k('1', { ctrlKey: true }),
      k('r'),
      k('ArrowLeft'),
      k('ArrowLeft', { ctrlKey: true, altKey: true }),
      k('Tab', { ctrlKey: true }),
      k('r', { ctrlKey: true, metaKey: true }),
    ]) {
      expect(shouldBlockKey(e, RELEASE)).toBe(false);
    }
  });

  it('blocks every browser key in release builds; development builds keep devtools and hard reload', () => {
    expect(shouldBlockKey(k('F5'), RELEASE)).toBe(true);
    expect(shouldBlockKey(k('F12'), RELEASE)).toBe(true);
    expect(shouldBlockKey(k('R', { ctrlKey: true, shiftKey: true }), RELEASE)).toBe(true);
    expect(shouldBlockKey(k('F5'), DEV)).toBe(true);
    expect(shouldBlockKey(k('r', { ctrlKey: true }), DEV)).toBe(true);
    expect(shouldBlockKey(k('p', { ctrlKey: true }), DEV)).toBe(true);
    expect(shouldBlockKey(k('F12'), DEV)).toBe(false);
    expect(shouldBlockKey(k('I', { ctrlKey: true, shiftKey: true }), DEV)).toBe(false);
    expect(shouldBlockKey(k('R', { ctrlKey: true, shiftKey: true }), DEV)).toBe(false);
    expect(shouldBlockKey(k('r', { metaKey: true }), MAC)).toBe(true);
  });
});

describe('native feel: pointer, menu and drag decisions', () => {
  it('blocks the back and forward mouse buttons, middle-click outside text fields and Ctrl+wheel only', () => {
    expect([0, 1, 2, 3, 4].map((b) => shouldBlockMouseButton(b))).toEqual([false, true, false, true, true]);
    expect([0, 1, 2, 3, 4].map((b) => shouldBlockMouseButton(b, true))).toEqual([false, false, false, true, true]);
    expect(shouldBlockWheel({ ctrlKey: true })).toBe(true);
    expect(shouldBlockWheel({ ctrlKey: false })).toBe(false);
  });

  it('keeps the native context menu only in text fields, offers Copy on selected text and blocks it elsewhere', () => {
    const none = { handled: false, editable: false, onSelection: false, developer: false };
    expect(contextMenuAction(none)).toBe('block');
    expect(contextMenuAction({ ...none, handled: true })).toBe('leave');
    expect(contextMenuAction({ ...none, handled: true, editable: true, onSelection: true })).toBe('leave');
    expect(contextMenuAction({ ...none, editable: true })).toBe('native');
    expect(contextMenuAction({ ...none, editable: true, onSelection: true })).toBe('native');
    expect(contextMenuAction({ ...none, onSelection: true })).toBe('copy');
    expect(contextMenuAction({ ...none, developer: true })).toBe('native');
  });

  it('stops files dropped where no Debrief target takes them from navigating the window', () => {
    expect(shouldBlockFileDrop({ files: true, handled: false })).toBe(true);
    expect(shouldBlockFileDrop({ files: true, handled: true })).toBe(false);
    expect(shouldBlockFileDrop({ files: false, handled: false })).toBe(false);
  });

  it('blocks dragging images and links but not Debrief drag sources or text', () => {
    expect(shouldBlockDrag({ tag: 'IMG', inLink: false, draggable: false })).toBe(true);
    expect(shouldBlockDrag({ tag: 'A', inLink: true, draggable: false })).toBe(true);
    expect(shouldBlockDrag({ tag: 'SPAN', inLink: true, draggable: false })).toBe(true);
    expect(shouldBlockDrag({ tag: 'IMG', inLink: false, draggable: true })).toBe(false);
    expect(shouldBlockDrag({ tag: 'DIV', inLink: false, draggable: false })).toBe(false);
    expect(shouldBlockDrag({ tag: null, inLink: false, draggable: false })).toBe(false);
  });

  it('knows text fields and editable content', () => {
    const make = (html: string) => {
      const host = document.createElement('div');
      host.innerHTML = html;
      return host.firstElementChild;
    };
    expect(isEditable(make('<input>'))).toBe(true);
    expect(isEditable(make('<input type="search">'))).toBe(true);
    expect(isEditable(make('<input type="checkbox">'))).toBe(false);
    expect(isEditable(make('<input disabled>'))).toBe(false);
    expect(isEditable(make('<textarea></textarea>'))).toBe(true);
    expect(isEditable(make('<textarea disabled></textarea>'))).toBe(false);
    expect(isEditable(make('<button>x</button>'))).toBe(false);
    expect(isEditable(null)).toBe(false);
    const ce = make('<div contenteditable="true">x</div>') as HTMLElement;
    // jsdom does not compute isContentEditable: model it.
    Object.defineProperty(ce, 'isContentEditable', { value: true });
    expect(isEditable(ce)).toBe(true);
  });
});

describe('native feel: the installed guard', () => {
  let host: HTMLElement;
  let remove: () => void;
  let copied: { x: number; y: number; text: string }[];

  beforeEach(() => {
    copied = [];
    host = document.createElement('div');
    host.innerHTML = '<button id="b">Tab</button><input id="i"><a id="a" href="#x"><img id="img" alt=""></a><p id="p">some log text</p><div id="d" draggable="true">tab</div>';
    document.body.appendChild(host);
    remove = installNativeFeel(window, RELEASE, { copyMenu: (x, y, text) => copied.push({ x, y, text }) });
  });
  afterEach(() => {
    remove();
    host.remove();
    window.getSelection()?.removeAllRanges();
  });

  const fire = <E extends Event>(target: EventTarget, e: E): E => {
    target.dispatchEvent(e);
    return e;
  };
  const el = (id: string) => host.querySelector(`#${id}`) as HTMLElement;

  it('prevents the browser default of reload keys but still lets Debrief see them', () => {
    const seen: string[] = [];
    const listener = (e: Event) => seen.push((e as KeyboardEvent).key);
    document.addEventListener('keydown', listener);
    const f5 = fire(el('b'), new KeyboardEvent('keydown', { key: 'F5', bubbles: true, cancelable: true }));
    const ctrlR = fire(el('b'), new KeyboardEvent('keydown', { key: 'r', ctrlKey: true, bubbles: true, cancelable: true }));
    const typed = fire(el('i'), new KeyboardEvent('keydown', { key: 'r', bubbles: true, cancelable: true }));
    document.removeEventListener('keydown', listener);
    expect(f5.defaultPrevented).toBe(true);
    expect(ctrlR.defaultPrevented).toBe(true);
    expect(typed.defaultPrevented).toBe(false);
    expect(seen).toEqual(['F5', 'r', 'r']);
  });

  it('suppresses the browser menu on chrome, keeps it in fields, offers Copy on selected text, and leaves Debrief menus alone', () => {
    const menu = (target: HTMLElement, init: MouseEventInit = {}) => fire(target, new MouseEvent('contextmenu', { bubbles: true, cancelable: true, ...init }));
    expect(menu(el('b')).defaultPrevented).toBe(true);
    expect(menu(el('i')).defaultPrevented).toBe(false);
    const range = document.createRange();
    range.selectNodeContents(el('p'));
    window.getSelection()?.addRange(range);
    expect(menu(el('p'), { clientX: 12, clientY: 34 }).defaultPrevented).toBe(true);
    expect(copied).toEqual([{ x: 12, y: 34, text: 'some log text' }]);
    expect(menu(el('b')).defaultPrevented).toBe(true);
    expect(copied).toHaveLength(1);
    window.getSelection()?.removeAllRanges();
    // Debrief's own menu: its handler prevents the default and the guard does not interfere.
    let opened = false;
    const own = (e: Event) => {
      e.preventDefault();
      opened = true;
    };
    el('b').addEventListener('contextmenu', own);
    expect(menu(el('b')).defaultPrevented).toBe(true);
    expect(opened).toBe(true);
    // A text node target counts as its element.
    expect(fire(el('p').firstChild!, new MouseEvent('contextmenu', { bubbles: true, cancelable: true })).defaultPrevented).toBe(true);
  });

  it('offers Copy only when the right-click is on the selected text, not on an ancestor such as body (regression)', () => {
    const menu = (target: Element) => fire(target, new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 1, clientY: 2 }));
    const range = document.createRange();
    range.selectNodeContents(el('p'));
    window.getSelection()?.addRange(range);
    expect(menu(document.body).defaultPrevented).toBe(true);
    expect(menu(host).defaultPrevented).toBe(true);
    expect(copied).toEqual([]);
    menu(el('p'));
    expect(copied).toHaveLength(1);
  });

  it('blocks Ctrl+wheel zoom without stopping Debrief handlers, and the back/forward mouse buttons', () => {
    let zoomed = false;
    el('b').addEventListener('wheel', () => (zoomed = true));
    const wheel = fire(el('b'), new WheelEvent('wheel', { ctrlKey: true, deltaY: 1, bubbles: true, cancelable: true }));
    expect(wheel.defaultPrevented).toBe(true);
    expect(zoomed).toBe(true);
    expect(fire(el('b'), new WheelEvent('wheel', { deltaY: 1, bubbles: true, cancelable: true })).defaultPrevented).toBe(false);
    expect(fire(el('b'), new MouseEvent('mouseup', { button: 3, bubbles: true, cancelable: true })).defaultPrevented).toBe(true);
    expect(fire(el('b'), new MouseEvent('mousedown', { button: 4, bubbles: true, cancelable: true })).defaultPrevented).toBe(true);
    expect(fire(el('b'), new MouseEvent('mouseup', { button: 0, bubbles: true, cancelable: true })).defaultPrevented).toBe(false);
    // Middle-click: no autoscroll on chrome; a text field keeps it (Linux pastes the primary selection).
    expect(fire(el('b'), new MouseEvent('mousedown', { button: 1, bubbles: true, cancelable: true })).defaultPrevented).toBe(true);
    expect(fire(el('i'), new MouseEvent('mousedown', { button: 1, bubbles: true, cancelable: true })).defaultPrevented).toBe(false);
  });

  it('keeps files dropped outside Debrief drop targets from navigating, and leaves handled drops alone', () => {
    const drag = (type: string, types: string[], handle = false) => {
      const e = new Event(type, { bubbles: true, cancelable: true }) as DragEvent;
      const dataTransfer = { types, dropEffect: 'copy' };
      Object.defineProperty(e, 'dataTransfer', { value: dataTransfer });
      if (handle) el('b').addEventListener(type, (x) => x.preventDefault(), { once: true });
      fire(el('b'), e);
      return { prevented: e.defaultPrevented, effect: dataTransfer.dropEffect };
    };
    expect(drag('dragover', ['Files'])).toEqual({ prevented: true, effect: 'none' });
    expect(drag('drop', ['Files']).prevented).toBe(true);
    expect(drag('dragover', ['text/plain'])).toEqual({ prevented: false, effect: 'copy' });
    expect(drag('dragover', ['Files'], true)).toEqual({ prevented: true, effect: 'copy' });
  });

  it('keeps the native menu on selected text when the app gives no copy menu', () => {
    remove();
    remove = installNativeFeel(window, RELEASE);
    const range = document.createRange();
    range.selectNodeContents(el('p'));
    window.getSelection()?.addRange(range);
    expect(fire(el('p'), new MouseEvent('contextmenu', { bubbles: true, cancelable: true })).defaultPrevented).toBe(false);
  });

  it('blocks dragging images and links out but not Debrief drag sources', () => {
    const drag = (target: HTMLElement) => fire(target, new Event('dragstart', { bubbles: true, cancelable: true }));
    expect(drag(el('img')).defaultPrevented).toBe(true);
    expect(drag(el('a')).defaultPrevented).toBe(true);
    expect(drag(el('d')).defaultPrevented).toBe(false);
    expect(fire(el('p').firstChild!, new Event('dragstart', { bubbles: true, cancelable: true })).defaultPrevented).toBe(false);
  });

  it('is removed again', () => {
    remove();
    expect(fire(el('b'), new KeyboardEvent('keydown', { key: 'F5', bubbles: true, cancelable: true })).defaultPrevented).toBe(false);
    remove = () => undefined;
  });
});

describe('provideNativeFeel', () => {
  it('installs the guard with the environment and removes it when destroyed', () => {
    const parent = TestBed.inject(EnvironmentInjector);
    const env = createEnvironmentInjector([provideNativeFeel()], parent);
    const press = () => {
      const e = new KeyboardEvent('keydown', { key: 'F5', bubbles: true, cancelable: true });
      document.body.dispatchEvent(e);
      return e.defaultPrevented;
    };
    expect(press()).toBe(true);
    env.destroy();
    expect(press()).toBe(false);
  });

  it('opens Debrief\'s menu with Copy on selected text', () => {
    const env = createEnvironmentInjector([provideNativeFeel()], TestBed.inject(EnvironmentInjector));
    const p = document.createElement('p');
    p.textContent = 'copy me';
    document.body.appendChild(p);
    const range = document.createRange();
    range.selectNodeContents(p);
    window.getSelection()?.addRange(range);
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    const e = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 5, clientY: 6 });
    p.dispatchEvent(e);
    const menu = TestBed.inject(ContextMenuService).menu();
    expect(e.defaultPrevented).toBe(true);
    expect(menu?.items.map((i) => i.label)).toEqual(['Copy']);
    expect([menu?.x, menu?.y]).toEqual([5, 6]);
    menu?.items[0]?.run();
    expect(writeText).toHaveBeenCalledWith('copy me');
    window.getSelection()?.removeAllRanges();
    p.remove();
    env.destroy();
  });
});
