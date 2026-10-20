# Angular UI (app/src/app)

- Presentation only; domain rules belong in the engine (e.g. highlight rules, profile TOML edits, regex validity are moving to Rust).
- Backend access: `core/backend.ts` (`ImtBackend` DI token; Tauri impl vs lazy mocks from `core/mock-registry`); feature backends similar (ai, jobs, browser, share, diagnostics, settings).
- Dock: `dock/` (dockview-based); panels are created via `angular-panel.ts` `PanelOutlet` attached to ApplicationRef; hidden panels are detached (their effects pause) — tests/specs must not assume hidden panels render (e.g. wait for the grid, not the timeline).
- Perf rules (docs/development.md "UI responsiveness"): FrameThrottle for pointer-driven signals, ConsumerGate for root stores, idle-debounced fetches, no per-row method calls in templates.
- Stores: `core/workspace.store.ts` is being split (R5) into query/data/timeline/prefs/theme; grid paging moving to a PagedRowsStore.
- Tests: Vitest specs next to sources; shared setup `test-setup.ts` loads mock backends (so `vi.mock('@tauri-apps/api/core')` has no effect — stub `__TAURI_INTERNALS__.invoke` instead); Playwright specs in `app/e2e/`, real-app wdio specs in `app/e2e-tauri/`.