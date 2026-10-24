import { Trigger } from './types';

// Attribution has no time window (Paddock rule 40, 2026-10-24): an input explains what it leads to,
// however long that takes on a busy machine. The first operation or user change after an input consumes it; whatever
// else happens in the same task (one click that pops a panel out and closes another) is the same action; anything in a
// later task without new input is the program's (a restore, the live tail, a timer).

/** Pointer travel that turns a press into a drag. */
const DRAG_PX = 8;

/** What the element a user acted on says about the trigger. Pure (DOM queries only). */
export function classifyTarget(target: EventTarget | null): Trigger {
  const el = target instanceof Element ? target : null;
  if (!el) return 'unknown';
  if (el.closest('debrief-command-palette, [data-activity-trigger="palette"]')) return 'palette';
  const inMenu = el.closest('[role="menu"], [role="menuitem"], [role="menuitemcheckbox"], [role="menuitemradio"], [role="menubar"]');
  // The application menu's dropdowns are overlays outside the title bar, marked with `data-menu`.
  if (inMenu) return el.closest('debrief-title-bar, debrief-menu-bar, [role="menubar"], [data-menu]') ? 'menu' : 'context-menu';
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
  /** The latest input and its sequence number (`inputs` when it happened). */
  private last: { trigger: Trigger; seq: number } | null = null;
  private press: { x: number; y: number; target: EventTarget | null; dragged: boolean } | null = null;
  /** The latest click in the native menu bar (an input too) and its sequence number. */
  private native: { id: string; seq: number } | null = null;
  private inputs = 0;
  /** Inputs up to this sequence number have explained an operation or change already. */
  private consumed = 0;
  /** The input consumed in the current task (other changes of the same task are the same action), else null. */
  private explained: Trigger | null = null;

  /** `endOfTask` runs a function once the current task is over (a zero-delay timer; tests pass a fake clock's). */
  constructor(private readonly endOfTask: (fn: () => void) => void = (fn) => void setTimeout(fn, 0)) {}

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
    this.consumed = this.inputs;
    this.explained = null;
  }

  /** Records an input (exposed for tests and for callers that know better). */
  note(trigger: Trigger): void {
    this.inputs++;
    this.last = { trigger, seq: this.inputs };
  }

  /** A click in the native (macOS) menu bar, which reaches the webview as an event, not as input. */
  noteNativeMenu(id: string): void {
    this.inputs++;
    this.native = { id, seq: this.inputs };
  }

  /** The trigger of an operation starting now: the latest input not yet explained by another, else `program`. */
  infer(): Trigger {
    if (this.native && this.native.seq > this.consumed && (!this.last || this.last.seq <= this.native.seq)) return 'native-menu';
    if (this.last && this.last.seq > this.consumed) return this.last.trigger;
    return this.explained ?? 'program';
  }

  /** The inputs so far have explained an operation or a change: what follows in a later task without new input is the program's. */
  consume(): void {
    const trigger = this.infer();
    this.consumed = this.inputs;
    if (trigger === 'program' || this.explained !== null) {
      if (trigger !== 'program') this.explained = trigger;
      return;
    }
    this.explained = trigger;
    this.endOfTask(() => (this.explained = null));
  }

  /** True when the native menu sent `operation` after an operation started with `inputsAtStart` inputs counted (the event may arrive late). */
  nativeMenuSent(operation: string, inputsAtStart: number): boolean {
    return !!this.native && this.native.id.endsWith(operation) && this.native.seq > inputsAtStart;
  }

  /** Counts user inputs (to tell whether the user did something since a point in time). */
  inputCount(): number {
    return this.inputs;
  }

  /** True when an input was explained in the current task (what else the task changes is the same action). */
  explainedThisTask(): boolean {
    return this.explained !== null;
  }

  /** True when there was user input that no operation or change has explained yet. */
  pendingInput(): boolean {
    return (!!this.last && this.last.seq > this.consumed) || (!!this.native && this.native.seq > this.consumed);
  }
}
