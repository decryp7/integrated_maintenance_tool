import { formatKeybinding } from '../../extensions/keybinding';
import { normalizeBinding } from '../../extensions/keybinding-overrides';
import { ImtCommand } from '../../extensions/tokens';

/** One line of the shortcut editor. */
export interface ShortcutRow {
  id: string;
  title: string;
  category: string;
  /** The effective binding, or null for none. */
  binding: string | null;
  /** The binding the command declares, or null for none. */
  defaultBinding: string | null;
  /** The effective binding as shown ("Ctrl+K"), empty for none. */
  keys: string;
  defaultKeys: string;
  /** The user changed (or removed) the declared binding. */
  changed: boolean;
  /** The command is only available in some situations (it has a `when`). */
  scoped: boolean;
}

/**
 * The rows of the editor: every command (bound or not) with its effective and declared binding, filtered by `query`
 * (every word must occur in the category, title or keys), sorted by category then title.
 */
export function shortcutRows(commands: readonly ImtCommand[], declared: (id: string) => string | undefined, query: string, uncategorised: string, mac?: boolean): ShortcutRow[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const canon = (b: string | null) => (b ? normalizeBinding(b) : null);
  return commands
    .map((c): ShortcutRow => {
      const binding = c.keybinding ?? null;
      const defaultBinding = declared(c.id) ?? null;
      return {
        id: c.id,
        title: c.title,
        category: c.category ?? uncategorised,
        binding,
        defaultBinding,
        keys: binding ? formatKeybinding(binding, mac) : '',
        defaultKeys: defaultBinding ? formatKeybinding(defaultBinding, mac) : '',
        changed: canon(binding) !== canon(defaultBinding),
        scoped: !!c.when,
      };
    })
    .filter((r) => {
      const hay = `${r.category} ${r.title} ${r.keys}`.toLowerCase();
      return words.every((w) => hay.includes(w));
    })
    .sort((a, b) => a.category.localeCompare(b.category) || a.title.localeCompare(b.title));
}
