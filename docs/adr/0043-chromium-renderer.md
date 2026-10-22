# 43. Research and the AI's browser modes drive a real Chromium; the docked browser is removed

- Status: accepted (supersedes [ADR 0024](0024-docked-browser.md), amends [ADR 0011](0011-internal-browser.md) and
  [ADR 0013](0013-ai-browsing-banner.md))
- Date: 2026-10-22

## Context

ADR 0024 put the AI's browser in a dockable panel over a native child web view (WebView2, WKWebView, WebKitGTK). In use:

- The system web views are not the browsers sites are tested against. WebView2 was served different Google pages than
  real Chrome (0 result links against 44 for the same collector), WebKitGTK differs again, and CAPTCHAs and consent
  walls show up more often.
- A web view cannot be driven reliably from outside: readiness had to be guessed from scripts evaluated with callbacks.
- The panel was small, needed overlay tracking (a native view draws above all HTML), key forwarding and per-platform
  geometry, and the user found it too small to be useful.

The user decided (2026-10-22) to drive a real Chromium over the DevTools protocol and to remove the panel.

## Decision

1. **A neutral renderer crate, `chromium-driver`** (`crates/chromium-driver`, no Debrief types, so other apps can copy it):
   - `discover`: Edge, Chrome or Chromium on Windows, macOS and Linux (`CHROMIUM_PATH` wins). Candidate lists are pure.
   - `launch`: a supervised child (own process group on Unix, `taskkill /T` on Windows) killed on drop, with a time cap
     (a watchdog) and a throwaway `--user-data-dir` per launch (or a persistent per-purpose profile when cookies must
     survive a challenge). Headful but off screen (`--window-position=-32000,-32000`); `set_visible` brings the window
     on screen and to the front, and moves it off again. Images, fonts and media are blocked while it is off screen.
     Downloads are denied.
   - `cdp`/`page`: a minimal synchronous DevTools client (`tungstenite`, MIT/Apache-2.0, loopback only): commands with
     replies, an event log to wait on, `Page.navigate`, `Runtime.evaluate`, history. A minimal client was chosen over
     `chromiumoxide` because the app's ports are synchronous, it avoids an async runtime and a large dependency tree, and
     only a dozen protocol calls are used.
   - `ready`: the readiness rule (load finished, results region present, DOM quiet; never network idle). `challenge`:
     consent walls, CAPTCHAs and "verify you are human" (pure, from URL, title and a text excerpt).
   - `install`: SHA-256 check, zip-slip-safe extraction with a size cap, executable lookup and a self-test.
   - **No stealth.** The real user agent, `--enable-automation` (Chrome's banner stays and `navigator.webdriver` is
     true). Nothing hides that the browser is automated.
2. **Optional pinned Chromium.** When no browser exists, or the user prefers a known one, Settings ▸ Web browser can
   download Chrome for Testing at a pinned version (`browser/runtime.rs`: URLs and SHA-256 per platform, verified,
   unpacked, self-tested). Only on request. An update installs beside the old version and switches only when the new
   one passes its self-test (rollback by never switching). Without any browser and without the download, research and the
   browser modes are unavailable, with a message that says what to do.
3. **Research and the chat modes use the renderer** through the unchanged `debrief_ai::browser` ports (`PageReader`,
   `ResultsReader`, `PageDriver`): the same page scripts are evaluated over the protocol and their replies pass the same
   one-shot gate (nonce, origin, size). Read along and Agent show the Chromium window on screen next to Debrief; research
   keeps it off screen. A challenge page comes to the front; when the user has dealt with it the work continues and the
   window goes back. The leak guard (`check_query`), drafts, citations, site rules and approvals are unchanged; site
   rules are enforced after a navigation (the page is sent back and the action fails) because the protocol client does
   not intercept requests.
4. **Links open in the user's default browser** (`browser_open`, `browser_open_suggested`) through the existing opener;
   no capability changed.
5. **The docked panel is removed**: view and dock registrations, menus, `browser_state/nav/close/place/open_external`,
   the child web view, geometry, key forwarding, the overlay tracker, the "web-profile" storage category, the
   `live-search-probe` feature, their i18n, mock, specs and e2e. No compatibility (pre-v1).

## It must be obvious when a page is driven by the AI

All indicators are outside the page. Nothing is injected into a page (no overlay, badge, frame, title change or
attribute): that could disturb page scripts and layout and leak into extracted text and screenshots, and a test asserts
that the page's DOM and text contain nothing added by Debrief.

1. **Chrome's automation banner** ("Chrome is being controlled by automated test software"), kept by `--enable-automation`.
2. **A distinct frame colour**: Debrief's accent in the title bar and tab strip, written into the throwaway profile's
   `Preferences` before launch (`browser.theme.user_color2` and friends). Best effort: a browser version that ignores it
   still shows the others.
3. **A window title** `"<product> AI — <reading|searching|waiting for you>: <host>"` (`cue::window_title`, product name
   from the app configuration). It is carried in the status chip's events; applying it to the OS window needs
   platform window APIs (a page's title cannot be changed without touching the page) and is left as a follow-up.
4. **A status chip in Debrief** while the AI drives: the activity ("Reading 3 of 5 results"), the host and a Stop button that
   ends the browser process at once (`browser_stop`). Accessible (`role=status`), localised.
5. **The research log** keeps listing every page opened and skipped (unchanged; the renderer feeds the same ports).

## Consequences

- One engine for research on every platform; pages are what Chrome users get.
- A browser must exist or be downloaded (about 200 MB, on request). Research is unavailable otherwise.
- Process hygiene is a tested property: kill on drop, kill on Stop, time cap, throwaway profile removed.
- Tests: pure unit tests for discovery, launch arguments, theme, title, readiness, challenge and install; an integration
  test launches a real browser when present against a local server with a script-rendered page; process-leak tests.
