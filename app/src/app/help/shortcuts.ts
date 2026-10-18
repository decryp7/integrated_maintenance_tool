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

const GENERAL = $localize`:@@help.group.general:General`;
const QUERY_BAR = $localize`:@@help.group.queryBar:Query bar`;
const TIMELINE = $localize`:@@help.group.timeline:Timeline`;
const EVENTS = $localize`:@@help.group.events:Events`;

export const FIXED_KEYS: readonly FixedKey[] = [
  { group: GENERAL, text: '/', description: $localize`:@@help.fixed.focusSearch:Focus the search box (when not typing)` },
  { group: GENERAL, binding: 'Escape', description: $localize`:@@help.fixed.close:Close the dialog, menu or popup` },
  { group: GENERAL, binding: 'Delete', description: $localize`:@@help.fixed.closeTab:Close the focused panel tab` },
  { group: QUERY_BAR, binding: 'Mod+Enter', description: $localize`:@@help.fixed.runQuery:Run the query` },
  { group: QUERY_BAR, binding: 'Ctrl+Space', description: $localize`:@@help.fixed.complete:Suggest completions` },
  { group: QUERY_BAR, text: 'Alt+↑ / Alt+↓', description: $localize`:@@help.fixed.history:Previous / next query from the history` },
  { group: TIMELINE, binding: 'Alt+Left', description: $localize`:@@help.fixed.prevRange:Previous time range` },
  { group: TIMELINE, binding: 'Alt+Right', description: $localize`:@@help.fixed.nextRange:Next time range` },
  { group: TIMELINE, text: 'Space + drag', description: $localize`:@@help.fixed.pan:Pan the timeline` },
  { group: EVENTS, text: '↑ ↓ PgUp PgDn Home End', description: $localize`:@@help.fixed.move:Move through the events` },
  { group: EVENTS, text: 'Shift+F10', description: $localize`:@@help.fixed.actions:Actions for the selected event` },
];

/** Builds the cheat sheet: every command with a key binding (by category) plus the fixed keys, filtered by `query`. */
export function shortcutGroups(commands: readonly ImtCommand[], fixed: readonly FixedKey[], query: string, mac?: boolean): ShortcutGroup[] {
  const groups = new Map<string, Shortcut[]>();
  const add = (group: string, item: Shortcut) => groups.set(group, [...(groups.get(group) ?? []), item]);
  const seen = new Set<string>();
  for (const c of commands) {
    if (!c.keybinding || seen.has(c.id)) continue;
    seen.add(c.id);
    add(c.category ?? GENERAL
, { keys: formatKeybinding(c.keybinding, mac), description: c.title });
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
