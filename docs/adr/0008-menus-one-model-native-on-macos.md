# 8. Menus: one model from the registries; in-app bar on Windows/Linux, native bar on macOS

- Status: accepted
- Date: 2026-10-17

## Context

The View menu had grown into one flat list of every registered view (about 40 entries, running
off the screen), every view with a `when` rule opened itself, and on macOS the app showed both
Tauri's default native menu bar (File Edit View Window Help) and IMT's own menu bar.

## Decision

- **One menu model.** `shell/menu/menu-model.ts` builds the whole menu bar as plain data with a pure
  function from the available commands, the registered views and the state behind check marks.
  Views declare a `category` (`core`, Analysis, Live, Profiling, Tools, Sharing, Diagnostics,
  Plugins, AI) and optionally `plugin` and `menu: 'palette'`; core views sit at the top of View and
  the others in a submenu per category; views without a category are plugin views (View ▸ Plugins,
  grouped by plugin). Commands the model does not place are reachable from the palette only, and
  every view from View ▸ Other windows… (the palette, filtered to views). Commands that are not
  registered are left out, so features appear in the menus when they land.
- **In-app bar** (Windows, Linux, browser): `@angular/cdk/menu` (ARIA menubar, keyboard,
  type-ahead, submenus, overlay placement), styled with IMT's tokens.
- **macOS native bar**: the UI sends the model (`set_app_menu`), Rust validates it (sizes, ids,
  labels; key bindings become accelerators), adds the standard macOS items (app menu, Edit with
  the webview's Undo..Select All, Window with Minimize/Zoom) and applies it, updating check marks in
  place when only states changed. Clicks go to the focused window as `imt://menu`. The in-app bar
  is hidden. The page sees shortcuts first; an echo of the same command from the menu bar within
  400 ms is ignored.
- **Auto-open is opt-in** (`ImtView.autoOpen`): `when` only decides whether a view is offered.
  The default layout is the core views; profiling views open with a profile, Tail status and
  Alerts when live tail starts unless the user closed them before.

## Consequences

- Plugins get a sensible place in the menus without code changes; the menu cannot list a view
  or command that does not exist, and the two menu bars never disagree.
- The wire contract of the native menu lives in `shell/menu/native-menu.types.ts` and
  `app/src-tauri/src/menu.rs`; change them together.
- Native menus can only be built on the macOS main thread, so their construction is verified in
  the real app; the spec (model → native entries), the diff and the routing are unit tested.
