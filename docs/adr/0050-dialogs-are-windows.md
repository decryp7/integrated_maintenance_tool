# 50. Dialogs are real OS windows

- Status: accepted (2026-10-23)
- Date: 2026-10-23
- Related: ADR 0016 (command table), the pop-out panels (`panel_windows.rs`), ADR 0043 (browser as its own window)
- Extraction: the window service (`DialogPresenter`, `DialogShell`, the Rust `dialog_windows` helper) is written to be
  extracted into `paddock-ui` and a small Paddock Rust crate later; the registry (`dialog-registry.ts`) and the dialog
  components stay in Debrief.

## Context

Dialogs were in-app overlays (`DialogService` + `debrief-dialog-host`). The user finds a popup inside the main window
unnatural: a first-start wizard, Settings or a model manager should be windows of the OS (own title bar, movable to
another monitor, remembered size, Cmd+W, taskbar/Mission Control behaviour). The rule applies to every Paddock app;
Debrief is first.

## Decision

Every dialog is an OS window owned by the Debrief window that opened it.

**Service.** `DialogService.open(Component, {title, inputs})` keeps its signature and returns a `DialogRef`
(`closed: Promise<result>`), so call sites did not change. A dialog component declares `static readonly dialogId`
and is listed once in `dialog-registry.ts` with its id, **modality**, size and lazy loader. The service picks a
`DialogPresenter`:

- `TauriDialogBackend` (desktop): `dialog_open` creates a `WebviewWindow` labelled `d-<host>-<n>`: native title bar with
  the dialog's title (and no title-bar buttons, see Title bar), owned by (child/transient/owner of) the opener, centred on it, at the size and place it had last
  time (kept on a visible monitor), minimum size, resizable. Yes/no confirmations (`ConfirmDialog`) are the native OS
  message box (`tauri-plugin-dialog`).
- `InPageDialogBackend` (browser build, e2e, overlays): the same dialogs rendered in the page by `debrief-dialog-host`;
  modal = blocking backdrop and focus trap, modeless = a floating panel that leaves the page usable.

If a window cannot be created the dialog is shown in the page (degrade, do not fail).

**Data flow.** `open(id, inputs) -> Promise<result>`: the typed input is validated in Rust (id shape, 8 MiB cap) and parked
in the `Dialogs` state; the dialog's own webview takes it with `dialog_take_input`. The result goes back with
`dialog_close(result)` (1 MiB cap) as the `debrief://dialog-result` event to the opener, followed by
`debrief://dialog-closed` when the window is gone (also when the user closed it without an answer). Nothing travels
through window globals. A dialog can open dialogs of its own (a confirmation from Settings); the answer goes to the
window that opened it. Inputs must be plain data (they cross to another window); callbacks stay in the opener and
are replaced by the result.

**Dialog window.** The app boots `#/dialog?id=..&host=..&label=..` into `DialogShell` (as pop-outs do): host-only services
are inert, UI state (query, selection, theme, display settings) syncs with the host over `UiSync` (the host adds
the dialog labels to its sync targets), data comes from the engine (`CurrentWindow` resolves `d-*` to the host's
workspace). Keys: Escape and Cmd/Ctrl+W are the dialog's Close or Cancel (the component may refuse through `static dialogCanClose`, e.g.
Settings with unapplied edits; a window-manager close request goes through the same check: Rust emits
`debrief://dialog-close-requested`, and closes anyway if the dialog does not answer within 1.5 s so a hung webview
cannot trap the user; a required-choice modal ignores keys and requests), Enter runs the default action (`[data-default]` or the first enabled primary button; not in
text areas, buttons, links or lists). On close the parent is re-enabled and focused.

**Bootstrap.** Each OS window is its own Angular bootstrap (Paddock rule 34). The main window and pop-outs keep the full
`appConfig` (now a lazily imported chunk, so a dialog window never loads it); a dialog window boots
`dialogAppConfig(id)`: logger, error handling, the backends (commands to the engine), icons, the theme and a command
forwarder, plus the *features* its registry entry names (`ai`, `tools`, `share`, `analysis`, `updates`, `diagnostics`,
`profiling`, `native`, `dotnet`), each only the backend providers of that area. It has no session, docking, windowing,
update, job or notification machinery and shares no store state with other windows: UI state syncs by events, data comes
from the engine, and commands a dialog runs (open a view, change the layout) are handed to its host (`dialogRun`).
`e2e/dialog-window.e2e.ts` opens the dialog route by URL in the browser build and renders 22 dialogs with only these
providers; a dialog that needs a backend its entry does not name fails there with a missing-provider error.

Measured (production build served locally, Chromium, median of 5, dialog route in a fresh context): JavaScript loaded
for the about / AI setup / settings / export dialogs 2574 / 2627 / 2575 / 2574 KB in 177-179 files with the full config,
2383 / 1847 / 1935 / 1906 KB in 68-104 files with the dialog config; time to a rendered dialog about 345 ms before, about
320 ms after (loopback, so the real saving on a slower disk or CPU is larger). JS heap showed no stable difference
(37-62 MB, noise). Real webview creation inside Tauri was not measured.

**Title bar.** Every dialog window, modal or modeless, is built with `closable(false)`, `minimizable(false)` and
`maximizable(false)` (`title_buttons`, checked by a test of `window_config`): no native close, minimise or zoom. Modeless windows stay
resizable and movable and remember their size and place. Each dialog therefore has a visible way out in its own content: a
Close or Cancel button, or its answer. The dialog shell guarantees it: when the content shows no Close/Cancel/Done/Not now button
(still loading, an error, a state that offers other actions) the shell adds a **Close** footer, and a test renders all 51
dialogs and asserts each has one. Escape and Cmd/Ctrl+W act as that button (Cancel on a modal). A modal without a Cancel
(`close-prompt`: a choice between two ways forward) is marked `requiredChoice` in the registry (sent to Rust as `required_choice`):
Escape, Cmd/Ctrl+W and a close request leave it open and are not even forwarded to it. A window-manager close request on any
other dialog (Linux, or Cmd+Q paths) is delivered to the dialog as its Cancel/Close, never a silent drop.

**Default size.** A dialog that streams or loads results (`ai-narrative`, `ai-proposal-result`, `ai-query`, `incident-summary`,
`research`, `whats-new`, ...) names a height in the registry instead of being fitted to its first (short) state; the default
size is bounded to 75% of the screen. Small decision dialogs omit the height and are fitted to their content; a fitted window only
ever **grows** (no re-fit cap; the engine stops at 90% of the screen height and the content scrolls inside), so a window never
shrinks under someone reading. A size the user set is remembered per dialog id and wins.

**About** shows the version the engine reports (`app_status`, the Cargo version; the dialog window has no lifecycle service of its own),
fills its window (the notices take the remaining height and scroll inside, the footer with "Copy version info..." and Close is
pinned) and gets the 15,639-line third-party notices from the engine (`legal_text`, compiled in), not from a generated TypeScript
module: the UI's JavaScript no longer contains them (the lazy chunk was about 781 KB of source text plus 13 KB of licences).

**One window per dialog id** (per opener): opening it again focuses the open window and returns the same ref (new
inputs are not applied to an open window).

**Modality** is an explicit per-dialog property in the registry (`modal | modeless`):

- *Modal* (answer needed before going on): the parent is disabled (`set_enabled(false)` plus the owner relation:
  Windows owner window, macOS child window, Linux transient), dimmed by a scrim in the page, and its commands and
  shortcuts are refused (`ModalGate`) while the window is open; the dialog stays above it. It is re-enabled when the
  last modal child closes.
- *Modeless* (a tool beside the work): owned (stays above the parent, minimises and hides with it) but the parent
  stays interactive and focus moves freely.

Classification (decisions in bold are judgement calls):

| Dialog id | Modality | Dialog | Why |
|---|---|---|---|
| `settings` | modeless | Settings | A tool kept beside the work; edits apply to the live app |
| `ai-setup` | **modal** | First-start AI setup | Only the *choice* of model is modal and quick. "Download and set up" closes the dialog at once and the rest is a background job (see below) |
| `ai-models` | modeless | Model manager | Long downloads with progress |
| `ai-query` | modeless | AI query dialog | The user compares it with the grid while writing |
| `ai-narrative / ai-proposal-result / ai-propose / ai-regex` | modeless | AI result and proposal windows | Read beside the data they refer to |
| `research / folder-setup` | modeless | Research, folder setup | Long-running, with their own progress |
| `whats-new / about / updates / cheat-sheet` | modeless | What's new, About, Check for updates, Keyboard shortcuts | Reference windows |
| `report-problem / collector` | modeless | Report a problem, diagnostics bundle | Own progress; the app stays usable |
| `export / redacted-copy` | modeless | Export events, Share redacted copy | Long work with its own progress; the parent stays usable |
| `profile-native / profile-etl / profile-dotnet` | modeless | Profiling launchers | Tools beside the work |
| `alert-rules / clock / manage-layouts` | modeless | Alert rules, Adjust clock, Manage perspectives | Edited while watching the grid |
| `unsaved-changes / close-prompt / open-ask` | modal | Unsaved changes, Close Debrief?, Open in this window? | The user must answer before anything else happens |
| `(ConfirmDialog)` | modal (native) | Every yes/no confirmation | The native OS message box (sheet on macOS); counts as a real window |
| `parser-editor / format-wizard` | **modeless** | Parser editor, format wizard | Tools edited beside the grid; the opener still awaits the result through the ref |
| `parser-import-confirm` | modal | Import confirmation | Feeds the next step; the caller awaits the answer |
| `package-review / plugin-permissions` | modal | Install plugin, plugin permissions | Consent: nothing else may change the plugin set meanwhile |
| `mark / drop / mask / extract / infer / marker-override` | modal | Rules created from a record | Act on a snapshot of the selected record; the parent must not change under them |
| `encoding / open-live / paste / import-range / recent-projects` | modal | Pickers feeding the next step (encoding, live tail, paste, time range, recent projects) | The time-range dialog gates the import that asked for it |
| `note-editor / knowledge-editor / incident-summary / search-web / report-export` | **modeless** | Editors and result windows | Kept beside the work; a modeless window can still answer its opener |
| `save-layout / add-tile / redaction` | modal | Small editors whose result the caller awaits | The caller continues with the result |
| `close-prompt` | modal, **required choice** | Close Debrief? | No Cancel: Escape and close requests are ignored |

**First-start AI setup is a modal choice plus a background download.** The dialog shows the hardware and the models that
fit; "Download and set up" calls `ai_setup_background` and closes. The engine runs one flow as Download jobs
(`ai/assistant_setup.rs`): the inference runtime when none is installed (phases Downloading, Unpacking, Finishing, Testing), then
the model (Downloading, Checking, Testing), with bytes, speed, time left and connections in Jobs and the status bar, and
"Setting up - 1.2 of 5.6 GB" in the assistant pane. The jobs are `quiet` (no generic job message); the flow posts exactly one
message itself: "AI assistant ready" with **Open the assistant** (`openAssistant`), or one plain failure message with **Retry**
(`setUpAssistant`, also the Retry of the job: it re-runs the whole set-up). Cancel is the job's Cancel in Jobs and is silent. A modal is
a quick decision, never a long wait. The time-range dialog is **modal** because the import that asked for it waits for the answer.

**Not dialogs (stay in the page):** context menus, dropdowns, autocomplete lists, tooltips, toasts and the command
palette / Search everything (`SearchEverythingPalette`: a transient overlay over the grid that is dismissed by the
first click away; a window would steal focus from the thing it filters and cost a webview per keystroke-sized
interaction). They have no `dialogId`, so they always use the in-page backend.

**Security.** Windows are created only by Rust from validated requests (`dialog_open` is callable from a Debrief
window or a dialog); `dialog_take_input` and `dialog_close` only from a dialog window and only for that window. A
new window class `DIALOG` in the command table (ADR 0016) gets the window commands plus the settings, storage and plugin
commands the dialogs need, and not the session, window-management, pop-out, drag-ghost or menu commands. The
capability `dialog.json` grants dialog webviews nothing beyond the default set; the CSP is unchanged and no remote
content is loaded. Per-dialog command allow-lists were not done here (the class was the boundary); ADR 0054 added them: a
dialog window may call only what its own declared contract names, and the `DIALOG` class only opens a command to contracts.

**Persistence.** Size and position per dialog id, in `localStorage` under `debrief.dialog.bounds` with a version;
missing, corrupt or foreign data falls back to the default placement.

**Performance.** A dialog window shows at once with its native title; its content shows a "Loading..." skeleton
until the dialog's chunk and input arrive, so no pre-warmed hidden window is needed. (Webview creation time was not
measured on a real desktop in this change: it needs a visible window, which tests must not open.)

## Consequences

- Dialogs are testable without a window: the in-page backend runs the e2e suite; unit tests cover both backends.
- A dialog opened from a dialog window works, but a dialog window has its own stores: state a dialog changes in a store
  that is not in `sharedUiState` is persisted through the engine and reloaded by the opener after the dialog closes.
- New dialogs are windows from the start: add `static readonly dialogId` and a registry entry; a test fails when they
  disagree.
