# 58. Start-up contract and window watchdog

- Status: Proposed (2026-10-24)
- Date: 2026-10-24
- Related: Paddock rules 5, 10, 12, 17, 19, 34 and 40; Paddock lessons "Never await IPC or the backend before the first
  paint" and "Never show an empty window while the UI loads"; ADR 0050 (dialog windows); ADR 0016 (one error contract)
- Applies to: every Paddock app with a webview (Debrief first; Pitcrew showed the same blank start)

## Context

Blank windows are a recurring bug kind across the family: Pitcrew's reopened window stayed blank (an initializer
awaited an IPC reply that never came), Debrief's dialog windows came up blank (missing providers), and now Debrief's
main window "sometimes loads blank" on macOS. In the blank sessions the Rust log shows the window and the native menu
being created and then nothing from the webview: no UI log line, no IPC, no "Workspace loaded". The screenshot shows a
half-painted frame (dockview's splitters, no activity bar, no status bar). In one session WebKit reported "the web
content process of window main terminated" after about 22 s and the reload started normally; the other blank sessions
were quit by the user after 10–15 s.

Findings on Debrief at `1dbd4b98` (browser build, Playwright WebKit and Chromium):

- No start-up path awaits IPC before the first paint: with every IPC reply withheld, or every call rejected, the shell
  (activity bar, status bar, dock) paints in full. The app initializers start their work and return.
- No layout feedback loop or start-up exception: 27 starts at 1920×1080, 1440×900, 2560×1440, 3008×1692 and odd
  sizes, device scale 1 and 2, and starts with the window resized back and forth across the narrow-window thresholds
  all load with no page error, ~60 frames/s and no DOM churn after load.
- The user runs a CPU limiter (App Tamer, auto-stop) that stops or throttles background processes. WKWebView's
  content process is a separate XPC process; when it is stopped mid-start the window keeps the last committed frame
  (the half-painted shell) and the app process hears nothing. Every webview app of the family is exposed: the user
  sees the same "sometimes blank" in Pitcrew.

Whatever stops the page — a paused or throttled content process, a lost first IPC message, a crash of the content
process, a future start-up exception — the app had no way to notice and the user's only way out was to quit. The
pattern (rule 40) is not one wrong await; it is that **a window's start is not a contract anyone checks**.

## Decision

Every window follows one start-up contract, with a UI half and a Rust half.

**UI (each OS window's bootstrap: main, workspace windows, pop-outs, dialogs).**

1. `index.html` carries a plain splash ("Starting Debrief…") styled by the global stylesheet (no inline style or script:
   CSP), fading in after 400 ms; Angular replaces it when the root component renders. A pop-out or dialog shell keeps
   it while its root element is swapped.
2. Nothing awaits IPC before the first paint. Initializers start work and return (already true; kept by review and by
   the e2e test below).
3. Start-up reads that the window depends on go through `retryUntilAnswered` (`core/startup/startup-contract.ts`): a
   short per-attempt timeout (4 s), up to 4 attempts, the first answer of any attempt wins, a rejection is an answer.
   There is no overall deadline (rule 12: a running start is never given up on). Only idempotent reads use it. Today:
   `window_ready` and the first `workspace_summary`.
4. After the first render (`afterNextRender`, render-driven, not timer-driven: a paused page reports when it resumes)
   the window calls `window_ready`. `provideStartupContract()` is one line in each window's config.
5. Uncaught errors, unhandled rejections and a failed bootstrap before ready are sent to the Rust log
   (`window_boot_error`: kind plus one bounded line, the message and first stack frame; never page text or file
   contents). After ready, Angular's error handler and the Logger take over.

**Rust (`app/src-tauri/src/startup_watchdog.rs`).**

1. `Builder::on_page_load` arms a watchdog for each of Debrief's own pages when it starts loading (first load and
   every reload); the internal browser's page and the drag ghost are not watched.
2. A background ticker (never the main thread) checks every 500 ms. A page that has not reported ready within 10 s
   is logged and **reloaded once**. If the reloaded page also misses the deadline, a native, window-owned dialog asks:
   **Reload / Restart in Safe Mode / Keep Waiting** (rule 17: a real OS window; rule 19: the repair never depends on
   the broken webview). Keep Waiting gives another full timeout.
3. A gap of more than 3 s between ticks means Debrief's own process was paused: the waiting windows' clocks restart
   instead of all firing on resume. The watchdog lives in the app process, so a paused content process cannot pause it.
4. When the content process terminates (`on_web_content_process_terminate`, macOS), the existing handler reloads the
   page (at most 3 times a minute) and marks the window. `window_ready` tells the reloaded page why it was reloaded
   (`terminated` or `unresponsive`), once, with the window's last UI state checkpoint, and the main window shows a
   calm notice: "The window's content stopped unexpectedly and was reloaded. Your open files and view are kept." The
   workspace lives in the engine, so the reloaded page gets the same sources; the query, filters, selection and the
   rest of the UI state come from the checkpoint (below); the dock layout and preferences from their local stores.

The decisions are pure (`Watchdog`: page started, ready, terminated, keep waiting, due actions at a time) and unit
tested with a passed-in clock; the ticker only applies them.

## Consequences

- A window that is paused, throttled, loses its first IPC or crashes during start recovers without the user: it is
  reloaded after 10 s, and only if that does not help is the user asked. The log always says which window, when and
  why, and carries the boot error if there was one.
- A window that is legitimately slow to paint its frame for more than 10 s is reloaded. The frame is painted before
  any IPC answer, so only a stopped process misses it; the 3 s pause rule avoids reloading after Debrief itself was
  stopped.
- A reloaded main window gets its whole UI state back (query, filters, selection, open views, saved queries, …) from
  `ui_checkpoint`: the page sends its UI state to Rust's memory 1 s after it changes (only once its own ready reply was
  handled, so a reloaded page's empty start never replaces it); `window_ready` returns it with the recovery reason,
  and the page applies it once the workspace is loaded. A user's own reload (no recovery) starts clean as before.
- Three new commands: `window_ready`, `window_boot_error` (`Quiet [WINDOW]`) and `ui_checkpoint` (`Quiet [MAIN]`, 16 MiB).
- Paddock: the contract is family-wide. The pure watchdog and the UI half (`retryUntilAnswered`, `reportReady`,
  boot-error forwarding, the splash) belong in the shared crate and `paddock-ui` once a second app adopts them
  (rule 39); Pitcrew is next. A family lesson records: "every window reports ready; Rust watches it".
- Start-up timings (2026-10-24, rule 16): the main window's `window_ready` is also its `windowReady` mark and starts
  the work the first window does not need (`after_first_paint`: the store worker, the tray icon; a 10 s fallback when
  no paint is reported, e.g. a start in the tray). The page reports `firstPaint`, `workspaceLoaded` and
  `firstInteractive` with `startup_mark` (`Quiet [MAIN]`), fire-and-forget: the contract above is unchanged, nothing
  awaits these calls, and their failures are ignored. See docs/performance.md.

## Tests

- `startup_watchdog` unit tests: ready in time; reload once then ask; the reloaded page's clock and its recovery
  reason; keep waiting; a 20 s pause of Debrief itself; a paused webview that resumes before the deadline; terminated
  → reported to the reloaded page; closed windows; several windows; watched labels; dialog answers; bounded error lines.
- Vitest: `retryUntilAnswered` (lost first reply, late answers, no deadline after the last attempt, rejections),
  `reportReady`, `provideStartupContract` (reports after the first render), boot error forwarding and its cap.
- Playwright (Chromium and WebKit), `e2e/startup-contract.e2e.ts`: the splash shows until Angular renders; with every
  IPC reply withheld (a paused content process) the frame paints and `window_ready` is sent, and the window recovers
  when the replies arrive; a reloaded window (terminated / unresponsive) shows its notice; a recovery reload brings
  back the query, the level filter and the selected event, and a user's own reload starts clean.
- Vitest `lifecycle/recovery.spec.ts`: restore after load, no restore on a normal start, checkpoints only after the
  reply was handled and debounced.
