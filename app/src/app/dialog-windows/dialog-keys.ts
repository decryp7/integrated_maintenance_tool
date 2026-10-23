/** The keys a dialog window answers (ADR 0050): Escape and Cmd/Ctrl+W close it, Enter runs its default action. */
export type DialogKeyAction = 'close' | 'default' | null;

export interface DialogKeyEvent {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  isComposing?: boolean;
  defaultPrevented?: boolean;
  target: unknown;
}

const OWN_ENTER = new Set(['TEXTAREA', 'BUTTON', 'A', 'SELECT', 'SUMMARY']);

function tagOf(target: unknown): string {
  return (target as { tagName?: string } | null)?.tagName?.toUpperCase() ?? '';
}

/** What a key press does in a dialog window. Enter keeps its own meaning in a text area, button, link or list. */
export function dialogKeyAction(e: DialogKeyEvent): DialogKeyAction {
  if (e.isComposing || e.defaultPrevented) return null;
  if (e.key === 'Escape' && !e.metaKey && !e.ctrlKey && !e.altKey) return 'close';
  if ((e.key === 'w' || e.key === 'W') && (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey) return 'close';
  if (e.key === 'Enter' && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
    const t = e.target as { isContentEditable?: boolean; getAttribute?: (n: string) => string | null } | null;
    if (OWN_ENTER.has(tagOf(e.target)) || t?.isContentEditable || t?.getAttribute?.('role') === 'option') return null;
    return 'default';
  }
  return null;
}

/** The dialog's default action: a `[data-default]` button, else the first enabled primary button. */
export function findDefaultAction(root: ParentNode): HTMLElement | null {
  return root.querySelector<HTMLElement>('button[data-default]:not([disabled])') ?? root.querySelector<HTMLElement>('button.primary:not([disabled]), button.btn.primary:not([disabled])');
}

/** The labels of a button that closes a dialog without an answer. */
const CLOSE_ACTION = /^(close|cancel|not now|dismiss|done|later|got it)\b/i;

/** Whether `root` has a visible button that closes the dialog (Close, Cancel, Not now, ...). */
export function hasCloseAction(root: ParentNode, except?: Element | null): boolean {
  return [...root.querySelectorAll('button')].some((b) => !except?.contains(b) && CLOSE_ACTION.test((b.textContent ?? '').trim()));
}
