# Shell baseline and the shell safety net (Paddock X50, W0 / M5)

`shell-baseline.e2e.ts` holds the screenshot baseline of the shell and workbench layer, taken before any code moves to
Paddock. The matrix is documented at the top of the spec: 30 screenshots per browser (Chromium and WebKit have their
own, named per project and platform in `shell-baseline.e2e.ts-snapshots/`, darwin only for now). Refresh with
`npx playwright test shell-baseline --update-snapshots` and review every PNG.

Known app nondeterminism, handled in the spec: the dock's first layout races the status bar and the data load, so the
Timeline/Events splitter ends 3 px off on a loaded machine (Chromium 300 or 303 px; WebKit 282 px). Each dock
screenshot has a `*.layout.txt` (group boxes); a load that differs is retried (up to 8) before pixels are compared. A
real regression differs on every try and fails. Masked: the status bar clock/time and the Jobs pane's elapsed times.

## Existing e2e specs that cover the shell and workbench layer (the M5 safety net)

- `dock.e2e.ts` - dock panels: layout save/restore, tabs, drag and drop, maximise, corrupt layout fallback.
- `dock-panels.e2e.ts` - panel open/close/activate through views and commands.
- `dock-float.e2e.ts` - floating groups and their layout.
- `dock-overflow.e2e.ts` - tab overflow in a narrow group.
- `dock-default-split.e2e.ts` - the default split sizes of the main layout.
- `dock-full-height.e2e.ts` - full-height panels (the inspector side).
- `dock-vs.e2e.ts` - Visual Studio style docking: auto-hide tool windows, pin, flyouts, full-height groups.
- `drag-ghost.e2e.ts` - the drag ghost and drop targets while moving panels.
- `popout.e2e.ts` - pop-out windows (window.open popups synced over a BroadcastChannel).
- `activity-bar.e2e.ts` - the activity bar: views, toggles, order.
- `perspectives.e2e.ts` - perspectives: switcher, Window menu, Ctrl+1..9, save/manage.
- `profiling-perspective.e2e.ts` - the Profiling perspective landing.
- `main-layout.e2e.ts` - main layout, panes and the health findings layout.
- `layout` family: `narrow-window.e2e.ts`, `home-narrow.e2e.ts` - regions and layout at narrow widths.
- `palette.e2e.ts` - the command palette. `commands.e2e.ts` - command registry and running commands.
- `menus.e2e.ts` - the application menu bar and menu behaviour (with `menu-support.ts`).
- `keyboard-regions.e2e.ts`, `keyboard-tab-stops.e2e.ts`, `ux-keyboard.e2e.ts` - keyboard regions and tab stops.
- `dialog-window.e2e.ts` - boots every dialog of the contract file in its own window bootstrap; Close/Escape/size rules.
- `dialog-modality.e2e.ts` - modal versus modeless dialog behaviour.
- `windowing.e2e.ts` - OS windows: state, centring, remembered size.
- `jobs.e2e.ts` - Jobs pane, toasts and Notifications.
- `service-health.e2e.ts` - the service health chip and Repair buttons; `tools-health.e2e.ts` - plugin health.
- `ai-chat.e2e.ts`, `ai-entry-points.e2e.ts` - the AI chat panel and its entry points into the shell.
- `settings.e2e.ts`, `settings-storage.e2e.ts` - the Settings window.
- `themes.e2e.ts` - four themes in both schemes, the main window and timeline screenshots (Chromium; win32 baselines).
- `a11y.e2e.ts`, `native-feel.e2e.ts`, `app-load.e2e.ts`, `i18n-pseudo.e2e.ts` - accessibility, native feel, startup, pseudo-locale.
