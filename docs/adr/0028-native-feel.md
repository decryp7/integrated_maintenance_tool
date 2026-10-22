# 28. Native feel: IMT's windows behave like a desktop application, not a web page

- Status: accepted (2026-10-21), amended by [ADR 0034](0034-native-window-capture.md) (the picture in an error snapshot is
  taken by the platform webview, not by rasterising the page) and [ADR 0035](0035-view-areas-and-perspectives.md) (`Alt+←`/`Alt+→`
  and the mouse's back/forward buttons are IMT's own history, not browser navigation)
- Related: ADR 0018 (themes), ADR 0024 (docked browser), `app/src/app/core/native-feel.ts`,
  `app/src-tauri/src/native_feel.rs`

## Context

IMT runs in a webview. Out of the box every label can be highlighted, a right-click anywhere shows the browser's menu
(Back, Reload, Save as, Print, Inspect), F5 reloads the window, Ctrl +/- zooms the page, middle-click starts
autoscroll, and a file dropped on a page without a handler navigates away. The user asked for the feel of a native
Windows/macOS/Linux application (WPF, Qt). A first step (October 2026) already switched off WebView2's accelerator
keys and blocked the browser menu outside text fields; this ADR records the whole policy.

## Decision

1. **Selection is opt-in.** The app shell is `user-select: none`, `cursor: default`, no tap highlight, no dragging of
   images or links (`-webkit-user-drag: none`), `overscroll-behavior: none`. Content the user may want to copy opts
   in with the `.imt-selectable` class (or is selectable by nature: text fields, `contenteditable`, `[role=textbox]`,
   grid cells, `pre`/`code`). Applied to: event rows and cells, the inspector (message, values, exception), the query
   editor, AI chat messages, notes, error details and code blocks.
2. **One context-menu policy** (`contextMenuAction`, pure): a menu IMT opened itself wins; text fields and editable
   content keep the platform's edit menu (cut, copy, paste, spelling); a right-click on selected text opens IMT's own
   menu with *Copy*; everywhere else nothing opens. Development builds show the browser menu (with *Inspect*) on
   Shift+right-click. The record menu (grid rows, inspector) gains *Copy* (the selection, else the messages).
3. **Browser keys and buttons are blocked** in every build: reload (F5, Ctrl/Cmd+R), print, save page, view source,
   browser find (F3, Ctrl/Cmd+F/G, unless an IMT handler already took the key and prevented its default, as the Find command does), page zoom (Ctrl +/-/0, Ctrl+wheel, pinch; IMT zooms
   its own views), back/forward (Alt+arrows, mouse buttons 4/5), middle-click autoscroll outside text fields. Release
   builds also block devtools and hard reload; development builds keep them. Only the browser's *default* is
   prevented, from a window listener in the bubble phase, so IMT's own shortcuts and handlers still see every event.
4. **The shell turns off what the webview lets it**: WebView2's `AreBrowserAcceleratorKeysEnabled` (release builds),
   `zoom_hotkeys_enabled(false)` and no browser extensions, for every IMT window (main and pop-outs). These switches
   exist on Windows (WebView2) only; on macOS and Linux only the UI guard applies. The guard covers the rest, so the
   browser build (`ng serve`, Playwright) behaves the same.
5. **Files dropped where no target takes them never navigate**: the guard swallows `dragover`/`drop` of files that no
   IMT drop target handled. In the desktop app Tauri's native drag-and-drop delivers drops to IMT's drop router (`core/dropped-files.ts`).
6. **Native typography and chrome**: a new appearance setting, *Interface font*, defaults to the operating system's
   UI font (Segoe UI Variable / Segoe UI on Windows, the San Francisco system font on macOS, `system-ui` with
   Cantarell / Ubuntu / Noto Sans on Linux); *Theme font* keeps the theme's own face (ADR 0018). Log text stays
   monospace. Focus rings show for keyboard focus (`:focus-visible`) only, never after a mouse click. Scrollbars are
   thin overlay-style bars drawn from theme tokens.
7. **External pages are untouched.** The docked browser (ADR 0024) is a separate webview showing web pages; neither
   the guard nor the window settings apply there, so selection and the page's context menu work normally.

## Consequences

- New UI that shows copyable content must add `.imt-selectable`; a component test asserts it for the main panels.
- The decision functions are pure and unit-tested; a Playwright test checks that right-clicking chrome is suppressed,
  that log text can be selected and the toolbar cannot.
- Screenshot baselines change once (system font by default).
