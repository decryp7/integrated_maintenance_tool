/**
 * Wire contract of the macOS native menu bar (camelCase JSON). Mirrored by the Rust DTOs in
 * `app/src-tauri/src/menu.rs`; change both together.
 *
 * Command: `set_app_menu { menu: NativeMenuModel }`. Event (to the focused window): `debrief://menu` with the item id.
 */

/** Event a native menu click is delivered as; the payload is the item id. */
export const MENU_EVENT = 'debrief://menu';

export type NativeMenuEntry =
  | {
      kind: 'item';
      id: string;
      label: string;
      /** Key binding in command syntax ("Mod+Shift+O"); the shell turns it into an accelerator. */
      keybinding: string | null;
      /** Null for a plain item; true/false for a toggle with a check mark. */
      checked: boolean | null;
      enabled: boolean;
    }
  | { kind: 'separator' }
  /** A disabled caption (a group label). */
  | { kind: 'header'; label: string }
  | { kind: 'submenu'; label: string; items: NativeMenuEntry[] };

export interface NativeTopMenu {
  /** Stable id: `file`, `edit`, `view`, ... The shell merges `edit` and `window` with the standard macOS items. */
  id: string;
  label: string;
  items: NativeMenuEntry[];
}

export interface NativeMenuModel {
  menus: NativeTopMenu[];
}
