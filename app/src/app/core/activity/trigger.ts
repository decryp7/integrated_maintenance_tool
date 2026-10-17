import { Trigger } from './types';

/** How long a user input explains an operation that starts after it. */
export const INPUT_WINDOW_MS = 1500;
/** How long a native menu click explains an operation. */
export const NATIVE_MENU_WINDOW_MS = 1000;
/** Pointer travel that turns a press into a drag. */
const DRAG_PX = 8;

/** What the element a user acted on says about the trigger. Pure (DOM queries only). */
export function classifyTarget(target: EventTarget | null): Trigger {
  const el = target instanceof Element ? target : null;
  if (!el) return 'unknown';
  if (el.closest('imt-command-palette, [data-activity-trigger="palette"]')) return 'palette';
  const inMenu = el.closest('[role="menu"], [role="menuitem"], [role="menuitemcheckbox"], [role="menuitemradio"], [role="menubar"]');
  // The application menu's dropdowns are overlays outside the title bar, marked with `data-menu`.
  if (inMenu) return el.closest('imt-title-bar, imt-menu-bar, [role="menubar"], [data-menu]') ? 'menu' : 'context-menu';
  if (el.closest('dialog, [role="dialog"], [role="alertdialog"]')) return 'dialog';
  if (el.closest('button, [role="button"], a[href], [role="tab"], input[type="checkbox"], input[type="radio"], select')) return 'button';
  return 'pointer';
}

/** A key press: in the palette or a dialog it confirms there; with a modifier (or a function key) it is a shortcut. */
export function classifyKey(e: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'target'>): Trigger {
  const where = classifyTarget(e.target);
  if (where === 'palette') return 'palette';
  if (where === 'dialog' && e.key === 'Enter') return 'dialog';
  if (where === 'context-menu' || where === 'menu') return where;
  if (e.ctrlKey || e.metaKey || e.altKey || /^F\d{1,2}$/.test(e.key)) return 'shortcut';
  const el = e.target instanceof Element ? e.target : null;
  const typing = !!el?.closest('input, textarea, [contenteditable="true"]');
  return typing ? 'keyboard' : 'shortcut';
}

/**
 * Remembers the latest user input in this window so an operation can say how it was started without every
 * call site passing a trigger. Listens in the capture phase, so it sees the input before any handler acts.
 */
export class TriggerTracker {
  private last: { trigger: Trigger; at: number } | null = null;
  private press: { x: number; y: number; target: EventTarget | null; dragged: boolean } | null = null;
  private native: { id: string; at: number } | null = null;
  private inputs = 0;

  constructor(private readonly now: () => number = () => Date.now()) {}

  /** Starts listening on `doc`; returns the function that stops. */
  install(doc: Document): () => void {
    const on = <K extends keyof DocumentEventMap>(type: K, fn: (e: DocumentEventMap[K]) => void) => {
      doc.addEventListener(type, fn, { capture: true, passive: true });
      return () => doc.removeEventListener(type, fn, { capture: true });
    };
    const offs = [
      on('keydown', (e) => {
        // Modifier keys alone and focus moves are not actions.
        if (!['Shift', 'Control', 'Alt', 'Meta', 'CapsLock', 'Tab'].includes(e.key)) this.note(classifyKey(e));
      }),
      on('pointerdown', (e) => (this.press = { x: e.clientX, y: e.clientY, target: e.target, dragged: false })),
      on('pointermove', (e) => {
        if (this.press && !this.press.dragged && Math.hypot(e.clientX - this.press.x, e.clientY - this.press.y) > DRAG_PX) this.press.dragged = true;
      }),
      on('pointerup', (e) => {
        this.note(this.press?.dragged ? 'drag-drop' : classifyTarget(e.target));
        this.press = null;
      }),
      on('click', (e) => {
        // Keyboard activation of a button fires click without a pointer.
        if (!this.press && e.detail === 0) this.note(classifyTarget(e.target));
      }),
      // Typing, pasting and IME input (a field filled without key events).
      on('input', (e) => this.note(classifyTarget(e.target) === 'palette' ? 'palette' : 'keyboard')),
      on('contextmenu', () => this.note('context-menu')),
      on('drop', () => this.note('drag-drop')),
    ];
    return () => offs.forEach((off) => off());
  }

  /** Forgets every input (tests). */
  reset(): void {
    this.last = null;
    this.native = null;
    this.press = null;
  }

  /** Records an input (exposed for tests and for callers that know better). */
  note(trigger: Trigger): void {
    this.last = { trigger, at: this.now() };
    this.inputs++;
  }

  /** A click in the native (macOS) menu bar, which reaches the webview as an event, not as input. */
  noteNativeMenu(id: string): void {
    this.native = { id, at: this.now() };
  }

  /** The trigger of an operation starting now: the latest input if recent, else `program`. */
  infer(): Trigger {
    const now = this.now();
    if (this.native && now - this.native.at <= NATIVE_MENU_WINDOW_MS && (!this.last || this.last.at <= this.native.at)) return 'native-menu';
    if (this.last && now - this.last.at <= INPUT_WINDOW_MS) return this.last.trigger;
    return 'program';
  }

  /** True when the native menu just sent `operation` (checked when an operation ends: the event may arrive late). */
  nativeMenuSent(operation: string, since: number): boolean {
    return !!this.native && this.native.id.endsWith(operation) && this.native.at >= since - NATIVE_MENU_WINDOW_MS;
  }

  /** Counts user inputs (to tell whether the user did something since a point in time). */
  inputCount(): number {
    return this.inputs;
  }

  /** True when there was user input within the input window. */
  recentInput(): boolean {
    return !!this.last && this.now() - this.last.at <= INPUT_WINDOW_MS;
  }
}
