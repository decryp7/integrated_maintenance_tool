import { DebriefActivityItem, DebriefActivityMenuItem } from '../extensions/tokens';
import { Perspectives, PERSPECTIVE_KEYS } from './perspectives';

/** Command ids the switcher's fly-out ends with. */
const SAVE_COMMAND = 'window.layout.save';
const MANAGE_COMMAND = 'window.layout.manage';

/** The fly-out entries: one per perspective (checked when active, with its Ctrl+n), then Save and Manage. */
export function perspectiveMenu(perspectives: Perspectives): DebriefActivityMenuItem[] {
  const active = perspectives.active().id;
  const entries = perspectives.list().slice(0, PERSPECTIVE_KEYS).map(
    (p, i): DebriefActivityMenuItem => ({
      id: `perspective:${p.id}`,
      label: p.name,
      icon: p.icon,
      checked: p.id === active,
      keybinding: `Mod+${i + 1}`,
      run: { command: `window.perspective.${i + 1}` },
    }),
  );
  return [
    ...entries,
    { id: 'perspective:save', label: $localize`:@@dock.perspective.menu.save:Save current as perspective…`, run: { command: SAVE_COMMAND } },
    { id: 'perspective:manage', label: $localize`:@@dock.perspective.menu.manage:Manage perspectives…`, run: { command: MANAGE_COMMAND } },
  ];
}

/** The perspective switcher in the activity bar: a fly-out of the perspectives with the active one checked. */
export function perspectiveActivityItems(perspectives: Perspectives): DebriefActivityItem[] {
  return [
    {
      id: 'perspectives',
      label: $localize`:@@dock.perspective.activity:Perspectives`,
      icon: 'columns',
      order: 150,
      run: { menu: () => perspectiveMenu(perspectives) },
    },
  ];
}
