# 54. Dialog kinds and declared contracts

- Status: Accepted and implemented in 1.1 (2026-10-24; user: per-dialog command lists, everything in 1.1; incident-summary becomes a modeless viewer; a saved layout that does not fit is fitted with one Undo notification; shared crates `paddock-dialogs` and `paddock-health`)
- Date: 2026-10-24
- Amends: ADR 0050 (dialogs are real OS windows). Related: ADR 0016 (command table), Paddock rules 17 and 40
- Extraction: the kind model, the contract and the checks are written to move into `paddock-ui` and the Paddock Rust
  crate with the rest of the window service; the registry entries stay in Debrief.

## Context

ADR 0050 made every dialog an OS window and a `DialogDef` (`app/src/app/dialog-windows/dialog-def.ts`,
`dialog-registry.ts`) carries only id, modality, width, an optional height/min size, `instanced`, `features` and a
loader. Everything else about a dialog is convention, and each convention has failed somewhere in the 51 dialogs:

- **Blank windows from missing providers.** A dialog that needs a backend its `features` list does not name boots with
  a missing-provider error and shows nothing. `e2e/dialog-window.e2e.ts` renders only 22 of the 51 dialogs, and the
  list is hand-kept.
- **A dialog denied the command it needs.** `resolve_close` is registered `Mutation [MAIN]` in `lib.rs` (the command
  table of ADR 0016); a dialog window is class `DIALOG` (`commands.rs`), so the close prompt's answer was refused. The
  command allow-list is per class (`dialog.json` grants nothing extra; the table is the only boundary) and nothing
  relates a dialog to the commands its component calls. ADR 0050 explicitly decided against per-dialog allow-lists;
  the bug shows the class is too coarse to check by eye.
- **Wrong sizes.** `DialogDef.width` is a number and `height` is usually absent, so the window is too tall for a
  three-line confirmation, too short for a form with a pinned footer, and "Explain event" (`ai-narrative`, 680 wide,
  no height) opens tiny, then the streamed text needs scrolling in a small box.
- **Missing Close buttons.** Native close/minimize/maximize are now disabled by decision, so a dialog without its own
  visible Close/Cancel cannot be closed with the mouse. Whether it has one is up to each component.
- **Modal vs modeless inconsistent.** Modality was decided per dialog by judgement ("bold are judgement calls" in
  0050); several modal dialogs (parser editor, format wizard, knowledge editor) can take minutes and block the parent,
  while quick pickers are modeless in other apps.

Per rule 40, five bug kinds from one family are a missing design rule, not five bugs: a dialog's kind decides
its size, buttons, modality and keyboard, and its dependencies are declared, not discovered at runtime.

User decisions this ADR encodes: all dialog windows have native close/minimize/maximize disabled and show a visible
Close/Cancel; Escape and Cmd/Ctrl+W close a modeless dialog and press Cancel on a modal one; a modal is a quick
decision, never a long wait (long work becomes a background job plus a notification); Settings is modeless; result
viewers open at about 680x560 and only grow.

## Decision

**1. Four kinds.** Every dialog declares exactly one `kind`; the kind is a closed enum and sets the defaults the
shell applies, so a dialog states only what differs.

| Kind | Modality | Size | Buttons | Keyboard |
|---|---|---|---|---|
| `decision` | modal | fits its content exactly (the shell measures the content, no scrollbars, max 60% of the host) | OK / Cancel (or labelled choices); the default button is marked | Enter = default, Escape and Cmd/Ctrl+W = Cancel |
| `form` | modal or modeless (declared) | fixed comfortable size (declared width x height), body scrolls, footer pinned | Primary / Cancel (modal) or Apply / Close (modeless) | Enter = default (not in text areas), Escape = Cancel (modal) or Close (modeless) |
| `viewer` | modeless | opens at 680x560; grows with streamed content up to 90% of the screen, never shrinks, remembers the larger size | Close (and Copy where the result is text) | Escape and Cmd/Ctrl+W = Close |
| `tool` | modeless | large (declared, default 960x680), remembers its size | Close; its own actions in the body | Escape and Cmd/Ctrl+W = Close (a `dialogCanClose` guard may refuse for unsaved edits) |

A modal is only a `decision` or a short `form`. A modal that starts work longer than a moment does not wait: it
returns its input, the opener starts a background job (rule 10) and a notification (ADR 0055) reports the outcome.
Native close, minimize and maximize are disabled for every kind; the shell draws the kind's footer buttons if the
component does not provide a `[data-dialog-close]` control (see lint 3), so a dialog can never be unclosable.

**2. One declared contract per dialog.** The registry entry becomes the whole contract (TypeScript is the source,
`dialog-registry.ts`; a generated JSON copy is checked into `app/src-tauri/` and compared by a Rust test, the same
way `generated_types.rs` mirrors `api.types.ts`):

```ts
defineDialog({
  id: 'ai-narrative', kind: 'viewer', instanced: true,
  input: type<NarrativeInput>(), result: type<void>(),   // typed; result is what the opener's promise resolves to
  backends: ['ai'],                                       // replaces `features`; the provider groups to boot
  commands: ['ai_explain_event', 'ai_status', 'ui_sync_*'],  // every IPC command the component may call
  load: () => import('../ai/dialogs/narrative-dialog'),
})
```

`DialogService.open` becomes generic over the declared input and result types (today `open(Component, {inputs})` is
untyped across the window boundary). The base commands every dialog needs (`dialog_take_input`, `dialog_close`,
`resolve_close` where relevant, UI sync, logging) are added by the kind, not repeated.

**3. Capabilities are derived, not hand-kept.** Rust creates dialog windows (`dialog_windows.rs`) and owns the command
table. Instead of the single `DIALOG` class, a dialog window is created with its dialog id and the guard
(`commands::guard`) allows a command from a dialog window only if the generated contract of *that* dialog lists it
(or a base command). The table stays the boundary (ADR 0016) and gets finer: `Mutation [MAIN | DIALOG]` is replaced by
the dialog contracts naming each command; `[MAIN]` commands cannot be reached by any dialog until a contract adds
them, with a reviewed diff. `resolve_close` is allowed for exactly `close-prompt`.

**4. Three checks make the contract real** (all deterministic, no sleeps, all in the affected-tests set):

1. *Boot-every-dialog test.* One Playwright spec iterates `DIALOG_DEFS` (not a hand list), opens the dialog route with
   the dialog's declared backends and a fixture input of its declared type (from a `fixture` field or the mock), and
   fails on a missing-provider error, an empty body, a console error, or a denied command (the mock backend
   enforces the contract's command list).
2. *Command lint.* A script (`app/scripts/check-dialog-commands.mjs`, run in `ng test` pre-step and CI) parses each
   dialog component and its injected backends for `invoke` command names and fails when one is not in the dialog's
   `commands`; the reverse check fails on a listed command nothing calls. A Rust test checks every listed command
   exists in the command table and is not `[MAIN]`-only.
3. *Close lint and test.* Every kind must have a visible Close/Cancel: the boot test asserts a `[data-dialog-close]`
   (or the shell footer) is visible and works, and Escape closes per kind; a size test asserts decision dialogs have
   no scrollbar, viewers open at 680x560 and the window never shrinks as content streams.

**5. Sizing is the shell's job.** `dialog_open` receives kind + declared size; Rust applies the minimum for the
kind (`decision` min = content, `viewer` min = 680x560, `tool` min = its declared minimum). A decision dialog sends
its measured content size once after first render (`dialog_resize`, validated and clamped in Rust); a viewer asks
to grow (`dialog_grow`, monotonic). Remembered bounds (`dialog-bounds.ts`) keep their key; a remembered size smaller
than the kind's minimum is raised to it.

### Dialogs and their kinds

| Dialog id | Today | Kind | Note |
|---|---|---|---|
| `unsaved-changes`, `close-prompt`, `open-ask` | modal | decision | `close-prompt` is the only dialog allowed `resolve_close` |
| `parser-import-confirm`, `marker-override`, `encoding`, `open-live`, `import-range`, `save-layout`, `recent-projects` | modal | decision | pickers feeding the next step; `import-range` gates the import |
| `package-review`, `plugin-permissions` | modal | decision | consent; content-fitted, install runs as a job |
| `drop`, `mask`, `extract`, `mark`, `infer` | modal | form (modal) | rules created from a record |
| `parser-editor`, `format-wizard`, `knowledge-editor`, `note-editor`, `redaction` | modal | form (modal) | the three biggest must not hold long work; test-parse runs as a job |
| `paste`, `add-tile`, `report-export`, `search-web` | modal | form (modal) | small editors; `report-export` returns its options and exports as a job |
| `clock`, `alert-rules`, `manage-layouts` | modeless | form (modeless) | edited while watching the grid |
| `export`, `redacted-copy`, `report-problem`, `collector` | modeless | form (modeless) | start a job with its own progress and may then stay as a status view |
| `ai-query`, `ai-propose`, `ai-setup`, `folder-setup` | modeless | form (modeless) | the setup wizards keep downloading in the background; closing never cancels the job |
| `profile-native`, `profile-etl`, `profile-dotnet` | modeless | form (modeless) | launchers |
| `ai-narrative`, `ai-proposal-result`, `ai-regex` | modeless | viewer | "Explain event" and the other results: 680x560, grow |
| `whats-new`, `about`, `cheat-sheet`, `updates` | modeless | viewer | reference; `updates` shows download progress inside |
| `incident-summary` | modal | viewer (modeless) | a result read beside the data; modal today without need |
| `settings` | modeless | tool | decided |
| `ai-models`, `research` | modeless | tool | long downloads / research session, own progress, remembered size |
| `(ConfirmDialog)` | native modal | decision (native) | stays the OS message box; the kind exists so tests treat it uniformly |

## Shared with the other Paddock apps (rule 39)

The family-level decision is Paddock ADR 0009 (dialogs as windows, kinds and declared contracts); Debrief is the first
adopter.

| Part | Where it lives |
|---|---|
| `DialogKind` enum and the per-kind defaults (size, buttons, keys), `defineDialog`, the typed `DialogService.open`, `DialogShell` with the visible Close/Cancel footer, `InPageDialogBackend`, `ModalGate`, bounds persistence (`dialog-bounds.ts`), the boot-every-dialog harness and the command lint script | `@decryp7/paddock-ui` (moved from `app/src/app/dialog-windows/` once the shapes settle; extraction already planned in ADR 0050) |
| Window creation by kind (min size, native close/min/max disabled, owner/modal relation, remembered bounds, `dialog_take_input`/`dialog_close`/`dialog_resize`/`dialog_grow`), the contract JSON schema and the per-dialog command guard | a shared Rust crate (`paddock-dialogs`, same set-up as `paddock-jobs`: no Tauri dependency in the core, a thin adapter feature for Tauri) |
| The 51 registry entries, their typed inputs and results, the `backends` provider groups, the command lists, the `close-prompt` -> `resolve_close` exception, the command table (`lib.rs`) | Debrief |

## Consequences

- A new dialog is one `defineDialog` call; the missing provider, the denied command, the missing Close button and the
  wrong size become failing tests at the moment the dialog is added, not bug reports.
- The command table gets stricter for dialogs: a command used by a dialog must be declared. This is more
  typing in the registry and a generated file to keep in step; it is the price of rule 40.
- `DialogDef` changes shape (`modality` and sizes derive from `kind`; `features` becomes `backends`). All 51 entries are
  rewritten once; component code is untouched except for removing hand-written footers and sizes.
- Several modal dialogs lose their ability to block on long work; some flows (test-parse in the parser editor) need
  a job and a result notification. This is deliberate.

## Migration plan

1. *Before 1.1 (docs and checks, no behaviour change).* Add `kind` to `DialogDef` with the table above, derive
   `modality` from it, add the registry test "every `DIALOG_DEFS` entry has a kind and a unique id". Make the boot spec
   iterate `DIALOG_DEFS` (fixes the 22-of-51 gap); fix whatever it finds (blank windows). Add the close lint and the
   visible-Close/Escape assertions; add a shell footer for dialogs without a close control.
2. *Before 1.1.* Shell sizing by kind: viewer minimum 680x560 and monotonic grow (fixes "Explain event"), decision
   fit-to-content, `form` fixed size with pinned footer; disable native close/min/max in `dialog_windows.rs`. Raise
   remembered sizes below the minimum.
3. *Before 1.1.* Add `commands` per dialog, the command lint, and the Rust test that every listed command exists. Move
   `resolve_close` to `[MAIN | DIALOG]` explicitly and record it as `close-prompt`'s.
4. *After 1.1.* Typed `input`/`result` in `DialogService.open`, generated contract JSON, and the per-dialog guard in
   `commands::guard`; remove the `DIALOG` class from the table as commands get declared.
5. *After 1.1.* Convert modal dialogs that start long work to job-plus-notification (parser editor test-parse,
   `format-wizard`, `knowledge-editor`, `redaction` preview) and lift the helpers to `paddock-ui`.

## Implementation notes (1.1)

Everything in the migration plan shipped together in 1.1, including the steps marked "after 1.1". Where the code differs
from the text above:

- **Where it lives.** The kind model, `defineDialog`, the contract types and the policy helpers are in
  `app/src/app/dialog-windows/kit/` (Angular only, no app imports: the folder moves to `paddock-ui` as it is). The Rust half is
  `dialog_contracts.rs` (parse and validate the contract file, `allows(dialog, command)`; no Debrief dependency, ready to be the
  `paddock-dialogs` crate) plus the window creation and the guard hook in `dialog_windows.rs` and `commands.rs`. The 51
  registry entries, `DIALOG_BASE_COMMANDS`, `DIALOG_SYNC_COMMANDS`, the host-only service list and the command table stay in
  Debrief. The shared packages are not created yet.
- **The `DIALOG` bit stays, its meaning changed.** The table's `DIALOG` flag no longer says "any dialog may call this"; it says
  "a dialog contract may name this". A command without it (`resolve_close`, `save_session`, window management) cannot be
  reached from any dialog whatever a contract says, and a Rust test fails when a contract names one. The guard allows a call from a
  dialog window only when the table opens the command to dialogs **and** that window's own contract (or the base set) names it.
  The dialog id comes from the `Dialogs` state by window label, never from the webview.
- **Rust owns kind and modality.** `dialog_open` takes no modality or required choice any more (the request type rejects them);
  it reads kind, modality, required choice, instancing and the minimum size from the contract, refuses an id with no contract,
  and refuses an instance key for a dialog that is not instanced. `dialog_fit_height` never goes below the contract's floor and
  never shrinks a viewer.
- **`resolve_close` stays MAIN-only.** The close prompt (and the open-ask dialog) already return their answer to the opener,
  which runs the command, so `close-prompt` needs no exception; the table above that said otherwise is superseded.
- **The contract file** is `app/src-tauri/dialog-contracts.json`, written by `DEBRIEF_WRITE_CONTRACTS=1 npx ng test
  --include=src/app/dialog-windows/dialog-contracts.spec.ts` and compared by the same spec on every run (the TS and Rust sides
  cannot drift). `commands` lists are not typed by hand: `scripts/check-dialog-commands.mjs` walks the type-checked TypeScript
  call graph from each dialog component (members, template references, imported components, injected classes, abstract
  backends to their implementations, store methods by use) and fails in both directions: a command a dialog reaches and does not
  declare, and a declared command nothing reaches. It also fails a command that is not in the table or that the table does not open to
  dialogs. Three things the walk does not enter are listed by hand with their reason: the synced UI state
  (`DIALOG_SYNC_COMMANDS`), host-only services (`HOST_ONLY_SERVICES`: the dock, the pop-out service, the session, and the two
  restarts that a dialog window hands to its host). The lint runs in `npm run lint`, as `pretest` and in CI.
- **Commands found mis-classed.** Settings' and the plugin windows' "Restart to apply", and "Restart as administrator" in the
  profiling and diagnostics windows, saved the session from a dialog window (`save_session` is MAIN-only and the session is the
  host's): the restart failed or reopened the dialog's empty session. A dialog window now hands both restarts to its host
  (`app.restartKeepingSession`, `app.restartElevated`) through `HOST_COMMAND_FORWARD`.
- **Typed results.** `DialogService.open(Component, options)` is typed from the registry: the answer is the entry's `result`, the
  inputs are the component's `input()` signals (an unknown input or a wrong answer type is a compile error). Only components that
  are not registered (`ConfirmDialog`, the in-page palette) use the untyped overload.
- **Long work in modals.** Parser editor, format wizard, knowledge editor, note editor, Search the web and Report export were made
  modeless in 1.0.2 on purpose (tools beside the grid) and stay `form` modeless, not `form` modal as the table above proposed. Their
  "test parse" reads a bounded head of one file (`read_file_head`, `preview_parse`), which is a quick preview, not a job. The
  redaction dialog is a modal form whose preview is one sample string and whose save is one profile write: short, so it stays
  modal. `ai-setup` is a modeless form as the table says (the choice closes the window and the download is a job).
- **Sizes.** `form` with no declared height is fitted to its content (grow-only) as before; it is not forced to a fixed height.
  The shell now measures the bottom of the last child, so its own Close footer is part of the fitted height (a decision that got the
  shell footer used to open with a scroll bar). A viewer's content scrolls inside while its Close stays pinned (What's new did not).
- **Not done.** The mock backend does not enforce the contract's command list (the mock backends are plain classes, not an
  `invoke` seam), so "a denied command" is caught by the lint and the Rust guard test, not by the boot spec. `dialog_resize` /
  `dialog_grow` as separate commands were not added; `dialog_fit_height` does both jobs per kind.

## Alternatives considered

- **Keep ADR 0050 and fix each dialog.** This is what produced the recurring bugs; rejected by rule 40.
- **Per-dialog Tauri capability files.** Tauri capabilities scope plugin permissions, not our own `invoke_handler`
  commands, so they cannot express this; the command table is where our commands are checked.
- **Infer the contract from the component by static analysis only.** Cannot see commands called through shared
  services; the declared list plus a lint in both directions is checkable.
- **One generic size per window.** The tiny "Explain event" and the tall confirmation both come from this.
