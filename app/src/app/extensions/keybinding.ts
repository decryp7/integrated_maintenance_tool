/** Is this a macOS-like platform, where "Mod" means Cmd instead of Ctrl? */
export function isMac(): boolean {
  return typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform ?? '');
}

/** Does the keyboard event match a binding such as "Mod+Shift+P" or "Alt+Left"? */
export function matchesKeybinding(binding: string, e: KeyboardEvent, mac = isMac()): boolean {
  const parts = binding.split('+').map((p) => p.trim().toLowerCase());
  const key = parts.pop();
  if (!key) return false;
  const wants = new Set(parts);
  const mod = wants.has('mod');
  const ctrl = wants.has('ctrl') || (mod && !mac);
  const meta = wants.has('meta') || (mod && mac);
  if (e.ctrlKey !== ctrl || e.metaKey !== meta || e.altKey !== wants.has('alt') || e.shiftKey !== wants.has('shift')) return false;
  const pressed = e.key.toLowerCase();
  return pressed === key || (key === 'left' && pressed === 'arrowleft') || (key === 'right' && pressed === 'arrowright');
}

/** Human-readable form of a binding for hints: "⌘⇧P" on macOS, "Ctrl+Shift+P" elsewhere. */
export function formatKeybinding(binding: string, mac = isMac()): string {
  const parts = binding.split('+').map((p) => p.trim());
  const names = mac
    ? { mod: '⌘', ctrl: '⌃', alt: '⌥', shift: '⇧', meta: '⌘' }
    : { mod: 'Ctrl', ctrl: 'Ctrl', alt: 'Alt', shift: 'Shift', meta: 'Win' };
  const out = parts.map((p) => {
    const lower = p.toLowerCase() as keyof typeof names;
    if (names[lower]) return names[lower];
    if (p.toLowerCase() === 'left') return '←';
    if (p.toLowerCase() === 'right') return '→';
    return p.length === 1 ? p.toUpperCase() : p;
  });
  return mac ? out.join('') : out.join('+');
}

/** `aria-keyshortcuts` value of a binding, e.g. "Control+Shift+O" ("Mod" is Meta on macOS, Control elsewhere). */
export function ariaKeyshortcuts(binding: string, mac = isMac()): string {
  return binding
    .split('+')
    .map((p) => (p.toLowerCase() === 'mod' ? (mac ? 'Meta' : 'Control') : p.toLowerCase() === 'ctrl' ? 'Control' : p))
    .join('+');
}
