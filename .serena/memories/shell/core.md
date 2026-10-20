# Tauri shell (app/src-tauri)

- `lib.rs` = composition root: `commands!` list, plugins, `on_run_event` (ExitRequested arms `lifecycle::arm_exit_watchdog`; Exit runs bounded `lifecycle::exit_teardown`).
- Per-window state: `windows.rs` (`WindowState`, `CurrentWindow` extractor, `read`/`write` helpers that run store work in spawn_blocking). The window-level store RwLock goes away with ADR 0014.
- Paths: `paths.rs` `PathGrants`/`GrantedPath` — webview paths must be granted by Rust-side sources (dialogs via `pick_paths`, drag-drop, CLI, recent files, project); one validator module.
- Main-thread work (tray, menus, window queries) only via `ui_dispatch.rs` (posts, never waits, never under a lock).
- Teardown: `lifecycle::teardown_step` bounds each step (3 s) and logs it; tray is skipped without a taskbar (session 0).
- Long work = Jobs (ADR 0012, `jobs/`): commands return a job id at once; notices through `notification_centre`.
- Activity logging: `activity/` (IPC classification tables; being replaced by a command table in R4).
- Debug-only test overrides: `dev_overrides.rs` (IMT_DATA_DIR relocates folders and gives the run its own app identifier; WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS; IMT_OPEN_PATHS).
- R4 (in progress) introduces: one command table/macro (kind, window class ACL, size limit), CmdError, typed DTOs generated to TS, services instead of commands calling commands, WindowScoped hooks, ConfigStore (ADR 0016).