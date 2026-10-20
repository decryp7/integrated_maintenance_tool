# 13. "AI is browsing" on the browser window

- Status: accepted
- Date: 2026-10-20

## Context

While a chat's agent mode or a research job drives the internal browser ([ADR 0011](0011-internal-browser.md)),
only the IMT window said so (the Web panel and the chat). The browser window itself looked the same as when
the user browses, so a user looking at it could not tell that the AI was in control, or stop it there.

The browser window is a plain `WebviewWindow` that shows only the remote page (`WebviewUrl::External`): it
has no capability, `browser::guard` refuses every command from it, and IMT never injects UI into the page
(a page could hide, imitate or click it). There is no IMT-owned surface in that window to put a banner on.

Options considered:

1. **A toolbar webview in the browser window** (Tauri's multiwebview: `Window::add_child`). Needs Tauri's
   `unstable` feature, which in `tauri-runtime-wry` 2.12 also changes how *every* window's webview is created
   (as a child with auto-resize bounds instead of the window's content), so all IMT windows and pop-outs
   would change behaviour. The browser window's isolation (no capability; commands refused by window label)
   would have to become per-webview, and `add_child` deadlocks on Windows when called from a synchronous
   command or an event handler.
2. **Injecting a banner into the page.** Rejected: the page could hide or fake it, and IMT keeps its UI out
   of untrusted pages.
3. **A small banner window glued to the browser window** (chosen), plus the window title and, on Windows 11,
   the window frame colour.

## Decision

1. **Who drives is Rust's state.** The browser hub already knows which chat (or research job,
   `research:<job>`) holds a window's browser; `BrowserHub::ai_driver` reports it as
   `AiDriver::Agent { sessionId }` or `AiDriver::Research`, and a hold whose Stop was pressed no longer counts.
   `driving::refresh` announces changes once (`imt://browser` event `{type: "driving", by}`) after every
   transition: an agent turn starts or ends, Stop, agent mode off, research starts or ends (finished,
   failed or cancelled) or is stopped, the chat is deleted, the browser or the IMT window closes.
   `browser_driving` returns the current driver.
2. **The banner** is a borderless window `aib-<owner>` with IMT's own UI (`index.html#/ai-browsing?host=…`,
   which boots only the banner component with the backends and logging). It is created the first time the AI
   drives, owned by the browser window (`parent`: on Windows it stays above the browser but not above other
   apps, hides when the browser is minimised and is destroyed with it; a child window that moves with it on
   macOS; transient on Linux), not resizable, not in the taskbar, does not take the focus, and covers the top
   40 logical pixels of the browser's client area. It follows the browser's move, resize and scale changes,
   and is hidden when the AI stops. Its Stop calls `browser_agent_stop` (with the chat) or
   cancels the research jobs (`jobs_cancel`; Jobs are the one cancel authority, ADR 0016) through the browser
   backend, like the chat and the Web panel. Its capability
   (`capabilities/ai-browsing.json`) is `core:default` and `log:default`; commands from it work on its IMT
   window's state (`CurrentWindow` resolves `aib-<owner>` to `<owner>`), and it is an auxiliary window (not
   listed, not restored, closing it does not close IMT).
3. **The title** of the browser window starts with "AI browsing — " while the AI drives
   (`driving::window_title`, also used for page title changes).
4. **The frame** turns amber on Windows 11 (`DwmSetWindowAttribute(DWMWA_BORDER_COLOR)`) and back to the
   system colour afterwards; older Windows refuses the attribute and nothing else changes.
5. Window changes run on a blocking thread, one at a time, and always apply the latest state, so they never
   build a window from a synchronous command and quick on/off changes converge.

## Consequences

- The user sees and can stop the AI from the browser window itself; the page never gets any of it.
- Where windows cannot be placed (Wayland), the banner may not sit exactly over the browser; the title, the
  Web panel and the chat still say it and have Stop.
- The banner hides the top 40 pixels of the page while the AI drives (the user is not meant to use the page
  then).
- One more window label scheme (`aib-*`) that the window manager and `CurrentWindow` know about.
