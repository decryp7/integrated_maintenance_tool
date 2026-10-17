import { formatKeybinding } from '../extensions/keybinding';
import { ImtCommand } from '../extensions/tokens';

/** One line of the cheat sheet. */
export interface Shortcut {
  /** As shown, e.g. "⌘⇧P" or "Ctrl+Shift+P". */
  keys: string;
  description: string;
}

export interface ShortcutGroup {
  title: string;
  items: Shortcut[];
}

/**
 * Keys the app handles outside the command registry (in a component's own key handling), described for the
 * cheat sheet. Bindings use the same syntax as commands; `text` is shown as is.
 */
export interface FixedKey {
  group: string;
  binding?: string;
  text?: string;
  description: string;
}

export const FIXED_KEYS: readonly FixedKey[] = [
  { group: 'General', text: '/', description: 'Focus the search box (when not typing)' },
  { group: 'General', binding: 'Escape', description: 'Close the dialog, menu or popup' },
  { group: 'General', binding: 'Delete', description: 'Close the focused panel tab' },
  { group: 'Query bar', binding: 'Mod+Enter', description: 'Run the query' },
  { group: 'Query bar', binding: 'Ctrl+Space', description: 'Suggest completions' },
  { group: 'Query bar', text: 'Alt+↑ / Alt+↓', description: 'Previous / next query from the history' },
  { group: 'Timeline', binding: 'Alt+Left', description: 'Previous time range' },
  { group: 'Timeline', binding: 'Alt+Right', description: 'Next time range' },
  { group: 'Timeline', text: 'Space + drag', description: 'Pan the timeline' },
  { group: 'Events', text: '↑ ↓ PgUp PgDn Home End', description: 'Move through the events' },
  { group: 'Events', text: 'Shift+F10', description: 'Actions for the selected event' },
];

/** Builds the cheat sheet: every command with a key binding (by category) plus the fixed keys, filtered by `query`. */
export function shortcutGroups(commands: readonly ImtCommand[], fixed: readonly FixedKey[], query: string, mac?: boolean): ShortcutGroup[] {
  const groups = new Map<string, Shortcut[]>();
  const add = (group: string, item: Shortcut) => groups.set(group, [...(groups.get(group) ?? []), item]);
  const seen = new Set<string>();
  for (const c of commands) {
    if (!c.keybinding || seen.has(c.id)) continue;
    seen.add(c.id);
    add(c.category ?? 'General', { keys: formatKeybinding(c.keybinding, mac), description: c.title });
  }
  for (const k of fixed) add(k.group, { keys: k.text ?? formatKeybinding(k.binding ?? '', mac), description: k.description });
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const matches = (group: string, s: Shortcut) => {
    const hay = `${group} ${s.description} ${s.keys}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  };
  return [...groups]
    .map(([title, items]) => ({ title, items: items.filter((s) => matches(title, s)).sort((a, b) => a.description.localeCompare(b.description)) }))
    .filter((g) => g.items.length > 0)
    .sort((a, b) => a.title.localeCompare(b.title));
}
